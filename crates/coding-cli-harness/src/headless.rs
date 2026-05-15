//! Drives one headless run end-to-end.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use tokio::sync::broadcast;
use tracing::warn;

use atomr_agents_coding_cli_core::{
    CliRequest, CliResult, CliRunId, CliVendor, CodingCliEvent, FinishReason, ToolCallRecord,
};
use atomr_agents_coding_cli_isolator::{IsolationOpts, Isolator};

use crate::error::Result;

pub(crate) async fn run_one(
    run_id: CliRunId,
    vendor: Arc<dyn CliVendor>,
    isolator: Arc<dyn Isolator>,
    req: CliRequest,
    event_tx: broadcast::Sender<CodingCliEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<CliResult> {
    // 1. Project atomr concepts onto the vendor's on-disk config.
    vendor
        .materialize_config(&req.project, &req.workdir)
        .await?;

    // 2. Build the vendor's headless command.
    let cmd = vendor.build_headless_command(&req, &req.workdir);

    // 3. Announce.
    let _ = event_tx.send(CodingCliEvent::RunStarted {
        run_id: run_id.clone(),
        vendor: vendor.kind(),
        model: req.model.clone(),
        session_id: None,
    });

    // 4. Spawn.
    let mut handle = isolator
        .spawn(
            cmd,
            IsolationOpts {
                capture_stdout: true,
                capture_stderr: true,
                grace: None,
            },
        )
        .await?;
    let mut stdout = handle
        .take_stdout()
        .expect("stdout requested via opts");
    let stderr = handle.take_stderr();

    // 5. Drive the parser. The vendor's parser sees each NDJSON
    // (or text) line. The harness accumulates a `CliResult` from
    // the normalized events as they come through.
    let mut parser = vendor.new_parser();
    let mut result = CliResult::new(run_id.clone(), vendor.kind());
    let mut pending_tools: std::collections::HashMap<String, ToolCallRecord> =
        std::collections::HashMap::new();

    while let Some(chunk) = stdout.recv().await {
        if cancel.load(Ordering::Relaxed) {
            result.finish_reason = FinishReason::Cancelled;
            let _ = handle.kill().await;
            break;
        }
        // Stdout is line-buffered by `pump_lines` in the isolator;
        // each chunk *should* be one line, but be defensive about
        // multi-line frames or partial lines.
        for line in std::str::from_utf8(&chunk).unwrap_or("").lines() {
            let events = match parser.parse_line(line) {
                Ok(evs) => evs,
                Err(e) => {
                    warn!(error = %e, "parser failed on line; emitting Note");
                    vec![CodingCliEvent::Note {
                        message: format!("parse error: {e}"),
                        fields: Default::default(),
                    }]
                }
            };
            for ev in events {
                accumulate(&mut result, &mut pending_tools, &ev);
                if let CodingCliEvent::RunFinished { reason, .. } = &ev {
                    result.finish_reason = *reason;
                }
                let _ = event_tx.send(ev);
            }
        }
    }

    // Flush.
    let trailing = parser.flush()?;
    for ev in trailing {
        accumulate(&mut result, &mut pending_tools, &ev);
        let _ = event_tx.send(ev);
    }

    // 6. Drain stderr (best-effort) and emit Notes so it's visible
    //    in the event log.
    if let Some(mut stderr) = stderr {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            while let Some(chunk) = stderr.recv().await {
                let text = String::from_utf8_lossy(&chunk).into_owned();
                let _ = event_tx.send(CodingCliEvent::Note {
                    message: format!("stderr: {}", text.trim_end()),
                    fields: Default::default(),
                });
            }
        });
    }

    // 7. Wait for the child.
    let status = handle.wait().await?;
    result.exit_code = status.code;
    result.ended_at = Some(Utc::now());

    // Any tool calls still "in flight" get finalized with a synthetic error.
    for (id, mut rec) in pending_tools.into_iter() {
        rec.error.get_or_insert("tool call did not finish".into());
        rec.finished_at = Some(Utc::now());
        result.tool_calls.push(rec);
        let _ = event_tx.send(CodingCliEvent::Note {
            message: format!("tool {} never reported a result", id),
            fields: Default::default(),
        });
    }

    if !status.success && result.finish_reason == FinishReason::Completed {
        result.finish_reason = FinishReason::ProcessError;
    }

    let _ = event_tx.send(CodingCliEvent::RunFinished {
        reason: result.finish_reason,
        result_text: Some(result.final_text.clone()),
    });

    Ok(result)
}

fn accumulate(
    result: &mut CliResult,
    pending: &mut std::collections::HashMap<String, ToolCallRecord>,
    ev: &CodingCliEvent,
) {
    match ev {
        CodingCliEvent::AssistantTextDelta { text } => {
            result.final_text.push_str(text);
        }
        CodingCliEvent::ToolCallStarted {
            tool_call_id,
            name,
            input,
        } => {
            pending.insert(
                tool_call_id.clone(),
                ToolCallRecord {
                    tool_call_id: tool_call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    output: None,
                    error: None,
                    started_at: Utc::now(),
                    finished_at: None,
                },
            );
        }
        CodingCliEvent::ToolCallFinished {
            tool_call_id,
            output,
            error,
        } => {
            if let Some(mut rec) = pending.remove(tool_call_id) {
                rec.output = output.clone();
                rec.error = error.clone();
                rec.finished_at = Some(Utc::now());
                result.tool_calls.push(rec);
            } else {
                result.tool_calls.push(ToolCallRecord {
                    tool_call_id: tool_call_id.clone(),
                    name: String::new(),
                    input: serde_json::Value::Null,
                    output: output.clone(),
                    error: error.clone(),
                    started_at: Utc::now(),
                    finished_at: Some(Utc::now()),
                });
            }
        }
        CodingCliEvent::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
        } => result.usage.add(*input_tokens, *output_tokens, *cost_usd),
        CodingCliEvent::RunFinished { result_text, .. } => {
            if let Some(t) = result_text {
                if !t.is_empty() {
                    result.final_text = t.clone();
                }
            }
        }
        _ => {}
    }
}

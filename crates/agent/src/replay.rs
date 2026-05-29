//! Side-effect-free replay (FR-1).
//!
//! [`ReplayProvider`] implements [`InferenceClient`] by serving the
//! *recorded* completion for each step instead of calling a live
//! provider. Construct an [`Agent`](crate::Agent) with a `ReplayProvider`
//! as its inference client and the model side of a run reproduces
//! byte-identically with **zero provider calls**. A missing record is a
//! loud, typed [`ReplayError`] — never a silent fall-back to live
//! inference.
//!
//! The companion [`record_turn`] converts a live [`TurnResult`] into an
//! [`InferenceRecord`] so a recording run can persist provenance through
//! a [`RecordingCheckpointer`](atomr_agents_state::RecordingCheckpointer).

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result, RunId, Value, WorkflowId};
use atomr_agents_state::{InferenceRecord, StepRecord, StepRecordStore, UsageRecord};
use atomr_agents_tool::{ParsedToolCall, Provider};
use atomr_infer_core::batch::ExecuteBatch;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};
use parking_lot::Mutex;
use thiserror::Error;

use crate::inference::{InferenceClient, TurnResult};

/// Errors raised on the replay path. Replay fails loudly rather than
/// re-inferring.
#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("replay: no record for step {step} of run {run} (have {available} steps)")]
    MissingStep {
        run: String,
        step: usize,
        available: usize,
    },
    #[error("replay: step {step} has no recorded inference")]
    NoInference { step: usize },
    #[error("replay: could not decode recorded value: {0}")]
    Decode(String),
}

impl From<ReplayError> for AgentError {
    fn from(e: ReplayError) -> Self {
        AgentError::Inference(e.to_string())
    }
}

/// An [`InferenceClient`] that returns recorded completions in order.
pub struct ReplayProvider {
    provider: Provider,
    run: String,
    steps: Vec<StepRecord>,
    cursor: Mutex<usize>,
}

impl ReplayProvider {
    /// Build a replay provider from recorded steps (ordered ascending by
    /// `super_step`). The provider discriminant is taken from the first
    /// recorded inference (defaulting to OpenAI if none is present —
    /// irrelevant for replay since no parsing of live deltas occurs).
    pub fn from_steps(run: impl Into<String>, steps: Vec<StepRecord>) -> Self {
        let provider = steps
            .iter()
            .find_map(|s| s.inference.as_ref())
            .and_then(|inf| provider_from_str(&inf.provider))
            .unwrap_or(Provider::OpenAi);
        Self {
            provider,
            run: run.into(),
            steps,
            cursor: Mutex::new(0),
        }
    }

    /// Load recorded steps for a run from a [`StepRecordStore`].
    pub async fn load(store: &dyn StepRecordStore, workflow: &WorkflowId, run: &RunId) -> Result<Self> {
        let steps = store.list_records(workflow, run).await?;
        Ok(Self::from_steps(run.as_str(), steps))
    }

    /// Number of recorded steps remaining to replay.
    pub fn remaining(&self) -> usize {
        self.steps.len().saturating_sub(*self.cursor.lock())
    }

    fn next_record(&self) -> std::result::Result<TurnResult, ReplayError> {
        let mut cur = self.cursor.lock();
        let idx = *cur;
        let step = self.steps.get(idx).ok_or(ReplayError::MissingStep {
            run: self.run.clone(),
            step: idx,
            available: self.steps.len(),
        })?;
        let inf = step
            .inference
            .as_ref()
            .ok_or(ReplayError::NoInference { step: idx })?;
        let turn = turn_from_record(inf)?;
        *cur += 1;
        Ok(turn)
    }
}

#[async_trait]
impl InferenceClient for ReplayProvider {
    fn provider(&self) -> Provider {
        self.provider
    }

    async fn run(&self, _batch: ExecuteBatch) -> Result<TurnResult> {
        // The batch is intentionally ignored: replay serves the recorded
        // completion and never calls a provider.
        Ok(self.next_record()?)
    }
}

/// Reconstruct a [`TurnResult`] from a recorded [`InferenceRecord`].
pub fn turn_from_record(inf: &InferenceRecord) -> std::result::Result<TurnResult, ReplayError> {
    let usage = TokenUsage {
        input_tokens: inf.usage.prompt_tokens,
        output_tokens: inf.usage.completion_tokens,
        reasoning_tokens: inf.usage.reasoning_tokens,
        cached_tokens: inf.usage.cached_tokens,
    };
    let finish_reason = match &inf.finish_reason {
        Some(s) => Some(
            serde_json::from_value::<FinishReason>(Value::String(s.clone()))
                .map_err(|e| ReplayError::Decode(format!("finish_reason '{s}': {e}")))?,
        ),
        None => None,
    };
    let tool_calls = inf
        .tool_calls
        .iter()
        .map(|v| {
            serde_json::from_value::<ParsedToolCall>(v.clone())
                .map_err(|e| ReplayError::Decode(format!("tool_call: {e}")))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(TurnResult {
        text: inf.raw_completion.clone(),
        usage,
        finish_reason,
        tool_calls,
    })
}

/// Capture a live [`TurnResult`] as an [`InferenceRecord`] for later
/// replay. `params_hash` / `prompt_hash` are supplied by the caller (they
/// derive from the request, which the pipeline holds).
pub fn record_turn(
    provider: Provider,
    model_id: impl Into<String>,
    model_version: impl Into<String>,
    params_hash: impl Into<String>,
    prompt_hash: impl Into<String>,
    turn: &TurnResult,
) -> InferenceRecord {
    InferenceRecord {
        provider: provider_to_str(provider).to_string(),
        model_id: model_id.into(),
        model_version: model_version.into(),
        params_hash: params_hash.into(),
        prompt_hash: prompt_hash.into(),
        raw_completion: turn.text.clone(),
        finish_reason: turn
            .finish_reason
            .and_then(|r| serde_json::to_value(r).ok())
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        usage: UsageRecord {
            prompt_tokens: turn.usage.input_tokens,
            completion_tokens: turn.usage.output_tokens,
            reasoning_tokens: turn.usage.reasoning_tokens,
            cached_tokens: turn.usage.cached_tokens,
        },
        tool_calls: turn
            .tool_calls
            .iter()
            .filter_map(|tc| serde_json::to_value(tc).ok())
            .collect(),
    }
}

fn provider_to_str(p: Provider) -> &'static str {
    match p {
        Provider::OpenAi => "open_ai",
        Provider::Anthropic => "anthropic",
    }
}

fn provider_from_str(s: &str) -> Option<Provider> {
    serde_json::from_value::<Provider>(Value::String(s.to_string())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_state::CheckpointKey;

    fn batch() -> ExecuteBatch {
        ExecuteBatch {
            request_id: "req".into(),
            model: "m".into(),
            messages: vec![],
            sampling: Default::default(),
            stream: false,
            estimated_tokens: 0,
        }
    }

    fn rec_step(run: &str, step: u64, text: &str, with_tool: bool) -> StepRecord {
        let tool_calls = if with_tool {
            vec![serde_json::to_value(ParsedToolCall {
                id: "c1".into(),
                name: "place_order".into(),
                arguments_raw: "{\"qty\":1}".into(),
            })
            .unwrap()]
        } else {
            vec![]
        };
        StepRecord::new(CheckpointKey {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from(run),
            super_step: step,
        })
        .with_inference(InferenceRecord {
            provider: "anthropic".into(),
            model_id: "claude".into(),
            model_version: "1".into(),
            params_hash: "p".into(),
            prompt_hash: "h".into(),
            raw_completion: text.into(),
            finish_reason: Some(if with_tool {
                "tool_calls".into()
            } else {
                "stop".into()
            }),
            usage: UsageRecord {
                prompt_tokens: 3,
                completion_tokens: 2,
                ..Default::default()
            },
            tool_calls,
        })
    }

    #[tokio::test]
    async fn replays_recorded_completions_in_order() {
        let steps = vec![rec_step("r", 0, "first", true), rec_step("r", 1, "second", false)];
        let rp = ReplayProvider::from_steps("r", steps);
        assert_eq!(rp.provider(), Provider::Anthropic);
        assert_eq!(rp.remaining(), 2);

        let t0 = rp.run(batch()).await.unwrap();
        assert_eq!(t0.text, "first");
        assert_eq!(t0.tool_calls.len(), 1);
        assert_eq!(t0.tool_calls[0].name, "place_order");
        assert_eq!(t0.finish_reason, Some(FinishReason::ToolCalls));

        let t1 = rp.run(batch()).await.unwrap();
        assert_eq!(t1.text, "second");
        assert!(t1.tool_calls.is_empty());
        assert_eq!(rp.remaining(), 0);
    }

    #[tokio::test]
    async fn missing_record_is_loud_error() {
        let rp = ReplayProvider::from_steps("r", vec![rec_step("r", 0, "only", false)]);
        let _ = rp.run(batch()).await.unwrap();
        // Second call: no more records -> typed error, never live inference.
        let err = rp.run(batch()).await.unwrap_err();
        assert!(matches!(err, AgentError::Inference(_)));
        assert!(err.to_string().contains("replay"));
    }

    #[test]
    fn record_turn_then_replay_is_identical() {
        let turn = TurnResult {
            text: "hello".into(),
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 7,
                reasoning_tokens: 1,
                cached_tokens: 2,
            },
            finish_reason: Some(FinishReason::Stop),
            tool_calls: vec![ParsedToolCall {
                id: "x".into(),
                name: "f".into(),
                arguments_raw: "{}".into(),
            }],
        };
        let rec = record_turn(Provider::Anthropic, "claude", "1", "p", "h", &turn);
        let back = turn_from_record(&rec).unwrap();
        assert_eq!(back.text, turn.text);
        assert_eq!(back.usage.input_tokens, 5);
        assert_eq!(back.usage.cached_tokens, 2);
        assert_eq!(back.finish_reason, Some(FinishReason::Stop));
        assert_eq!(back.tool_calls, turn.tool_calls);
    }
}

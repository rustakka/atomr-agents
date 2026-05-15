//! Starts an interactive CLI session wrapped in a tmux pane and
//! returns an `InteractiveSessionHandle`.
//!
//! The two-step dance is:
//! 1. `tmux new-session -d -s <id> -c <workdir> <CLI> <args...>`
//!    (headless via the isolator) — creates the tmux session and
//!    starts the CLI inside it.
//! 2. `tmux attach-session -t <id>` (PTY via the isolator) — opens an
//!    attached client whose stdin/stdout we own. The PTY pump
//!    forwards bytes ↔ session channels.
//!
//! Both calls go through the same `Isolator`, so Docker isolation is
//! transparent: tmux runs *inside* the container.

use std::sync::Arc;

use atomr_agents_coding_cli_core::{
    CliCommand, CliRequest, CliSessionId, CliVendor,
};
use atomr_agents_coding_cli_isolator::{IsolationOpts, Isolator};

use crate::error::{HarnessError, Result};
use crate::pty_pump;
use crate::session::InteractiveSessionHandle;

const TMUX_BIN: &str = "tmux";

pub(crate) async fn start_session(
    id: CliSessionId,
    vendor: Arc<dyn CliVendor>,
    isolator: Arc<dyn Isolator>,
    req: CliRequest,
) -> Result<Arc<InteractiveSessionHandle>> {
    // 1. Materialize concept projection.
    vendor.materialize_config(&req.project, &req.workdir).await?;

    // 2. Build the inner CLI command.
    let inner = vendor.build_interactive_command(&req, &req.workdir);

    // 3. tmux new-session (headless spawn).
    let tmux_session = format!("atomr-cli-{}", id.as_str());
    let mut create = CliCommand::new(TMUX_BIN, req.workdir.clone())
        .arg("new-session")
        .arg("-d")
        .arg_pair("-s", tmux_session.clone())
        .arg_pair("-c", req.workdir.to_string_lossy().into_owned());
    create = create.arg(inner.program.as_os_str());
    for a in &inner.args {
        create = create.arg(a);
    }
    // Inherit any env requested by the inner CLI.
    for (k, v) in &inner.env {
        create = create.envv(k, v);
    }

    let mut create_handle = isolator
        .spawn(
            create,
            IsolationOpts {
                capture_stdout: false,
                capture_stderr: true,
                grace: None,
            },
        )
        .await?;
    let create_status = create_handle.wait().await?;
    if !create_status.success {
        return Err(HarnessError::InvalidRequest(format!(
            "tmux new-session failed with exit code {:?}; check that tmux is installed in the isolator",
            create_status.code
        )));
    }

    // 4. tmux attach-session (PTY spawn). The attaching process is
    //    what the operator's WS client mirrors.
    let attach = CliCommand::new(TMUX_BIN, req.workdir.clone())
        .arg("attach-session")
        .arg_pair("-t", tmux_session.clone())
        .with_pty();
    let attach_handle = isolator.spawn(attach, IsolationOpts::default()).await?;

    // 5. Spawn the byte pump and return the handle.
    let pumps = pty_pump::spawn(attach_handle);
    let handle = InteractiveSessionHandle {
        id: id.clone(),
        vendor: vendor.kind(),
        tmux_session,
        started_at: chrono::Utc::now(),
        request: req,
        events: pumps.events_tx,
        input: pumps.input_tx,
        closed: pumps.closed,
    };
    Ok(Arc::new(handle))
}

/// Stop a tmux-wrapped session. Kills the tmux session so any
/// still-attached operators get disconnected.
pub(crate) async fn stop_session(
    isolator: Arc<dyn Isolator>,
    tmux_session: &str,
    workdir: std::path::PathBuf,
) -> Result<()> {
    let cmd = CliCommand::new(TMUX_BIN, workdir)
        .arg("kill-session")
        .arg_pair("-t", tmux_session);
    let mut h = isolator
        .spawn(
            cmd,
            IsolationOpts {
                capture_stdout: false,
                capture_stderr: false,
                grace: None,
            },
        )
        .await?;
    let _ = h.wait().await;
    Ok(())
}

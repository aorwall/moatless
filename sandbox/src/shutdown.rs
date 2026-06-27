//! Graceful shutdown handling for the sandbox runtime.
//!
//! The sandbox runs one agent per pod. Without signal handling the process
//! blocks on `axum::serve` and is SIGKILLed (exit 137) at the end of the pod's
//! ~30s termination grace period on *every* teardown — idle-down, node drain,
//! eviction, and PR-close. That churn is the source of the exit-137 kills seen
//! in production. Installing a SIGTERM/SIGINT handler that stops the agent and
//! cancels the lifecycle token lets the process drain and exit promptly.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::agent::agent_state::AgentState;
use crate::state::SandboxState;

/// Side effects to run when a shutdown signal arrives: stop the running agent
/// (best-effort), flush the Claude transcript to S3, then cancel the lifecycle
/// token so background tasks and both servers wind down.
///
/// Extracted from the signal handler so it can be unit-tested without raising a
/// real OS signal. `state` is optional purely so the agent-stop/token-cancel
/// behavior can be exercised in isolation; production always passes `Some`.
pub async fn perform_shutdown(
    agent: &AgentState,
    lifecycle_cancel: &CancellationToken,
    state: Option<&SandboxState>,
) {
    // Cancel the in-flight turn so the Claude CLI subprocess is killed and the
    // turn ends now, instead of being SIGKILLed at grace-period end.
    // `AgentNotRunning` (an idle agent) is expected on a routine idle-down and
    // is ignored.
    match agent.stop().await {
        Ok(()) => info!("Shutdown: stopped running agent"),
        Err(e) => info!(error = %e, "Shutdown: no running agent to stop (continuing)"),
    }

    // Flush the transcript AFTER the agent has stopped (so the active `.jsonl` is
    // flushed by the CLI) and BEFORE cancelling the lifecycle token (which stops
    // the periodic sync). On a graceful kill — including a spot reclaim's 2-min
    // drain → SIGTERM — this persists the final delta with no loss. Bounded
    // internally so it can't blow the pod's 30s grace period.
    if let Some(state) = state {
        crate::init::transcript_sync::flush_on_shutdown(state).await;
    }

    lifecycle_cancel.cancel();
}

/// Await the first SIGTERM or SIGINT.
///
/// Resolves as soon as either signal is delivered. SIGTERM is what Kubernetes
/// sends on pod termination; SIGINT covers `Ctrl-C` in local runs.
pub async fn await_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to install SIGTERM handler; falling back to Ctrl-C");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to install SIGINT handler; SIGTERM only");
            sigterm.recv().await;
            return;
        }
    };

    tokio::select! {
        _ = sigterm.recv() => info!("Received SIGTERM"),
        _ = sigint.recv() => info!("Received SIGINT"),
    }
}

/// Spawn the shutdown driver: wait for a signal, then run [`perform_shutdown`].
///
/// Takes a clone of the whole [`SandboxState`] so the transcript flush can reach
/// the backend client and config. `main`'s `state` stays intact for the servers,
/// which observe the same lifecycle token via graceful shutdown.
pub fn spawn_shutdown_driver(state: SandboxState) {
    tokio::spawn(async move {
        await_shutdown_signal().await;
        info!("Shutdown signal received — stopping agent and winding down");
        let agent = Arc::clone(&state.agent);
        let lifecycle_cancel = state.lifecycle_cancel.clone();
        perform_shutdown(&agent, &lifecycle_cancel, Some(&state)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn perform_shutdown_cancels_token_when_agent_idle() {
        let agent = AgentState::new();
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        // Idle agent → stop() errors (AgentNotRunning), but shutdown still
        // cancels the lifecycle token so the servers and background tasks drain.
        perform_shutdown(&agent, &token, None).await;
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn perform_shutdown_stops_running_agent_and_cancels_token() {
        let agent = AgentState::new();
        let agent_cancel = CancellationToken::new();
        agent.set_running(agent_cancel.clone()).await;
        assert!(!agent_cancel.is_cancelled());

        let token = CancellationToken::new();
        perform_shutdown(&agent, &token, None).await;

        // The agent's in-flight turn was cancelled...
        assert!(agent_cancel.is_cancelled());
        // ...and the lifecycle token was cancelled.
        assert!(token.is_cancelled());
    }
}

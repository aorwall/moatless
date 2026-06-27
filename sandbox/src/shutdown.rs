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

/// Side effects to run when a shutdown signal arrives: stop the running agent
/// (best-effort) and cancel the lifecycle token so background tasks and both
/// servers wind down.
///
/// Extracted from the signal handler so it can be unit-tested without raising a
/// real OS signal: it takes only the [`AgentState`] and [`CancellationToken`].
pub async fn perform_shutdown(agent: &AgentState, lifecycle_cancel: &CancellationToken) {
    // Cancel the in-flight turn so the Claude CLI subprocess is killed and the
    // turn ends now, instead of being SIGKILLed at grace-period end.
    // `AgentNotRunning` (an idle agent) is expected on a routine idle-down and
    // is ignored.
    match agent.stop().await {
        Ok(()) => info!("Shutdown: stopped running agent"),
        Err(e) => info!(error = %e, "Shutdown: no running agent to stop (continuing)"),
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
/// Cloning the `Arc<AgentState>` and the token keeps `main`'s `state` intact for
/// the servers, which observe the same token via graceful shutdown.
pub fn spawn_shutdown_driver(agent: Arc<AgentState>, lifecycle_cancel: CancellationToken) {
    tokio::spawn(async move {
        await_shutdown_signal().await;
        info!("Shutdown signal received — stopping agent and winding down");
        perform_shutdown(&agent, &lifecycle_cancel).await;
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
        perform_shutdown(&agent, &token).await;
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn perform_shutdown_stops_running_agent_and_cancels_token() {
        let agent = AgentState::new();
        let agent_cancel = CancellationToken::new();
        agent.set_running(agent_cancel.clone()).await;
        assert!(!agent_cancel.is_cancelled());

        let token = CancellationToken::new();
        perform_shutdown(&agent, &token).await;

        // The agent's in-flight turn was cancelled...
        assert!(agent_cancel.is_cancelled());
        // ...and the lifecycle token was cancelled.
        assert!(token.is_cancelled());
    }
}

//! Graceful shutdown handling for the sandbox runtime.
//!
//! The sandbox runs one agent per pod. Without signal handling the process
//! blocks on `axum::serve` and is SIGKILLed (exit 137) at the end of the pod's
//! ~30s termination grace period on *every* teardown — idle-down, node drain,
//! eviction, and PR-close. That churn is the source of the exit-137 kills seen
//! in production. Installing a SIGTERM/SIGINT handler that stops the agent,
//! flushes CLI session state, and cancels the lifecycle token lets the process
//! drain and exit promptly.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::agent::agent_state::AgentState;
use crate::state::SandboxState;

/// Bounded time budget for draining the event send buffer on graceful shutdown.
///
/// Kept small so the total teardown stays within the pod's 30s termination grace
/// period. The bounded steps run strictly in sequence, so they add up:
///
/// | step | budget |
/// |---|---|
/// | this drain | 5s |
/// | `transcript_sync::flush_on_shutdown` | 12s |
/// | `workspace_sync::flush_on_shutdown` | 10s |
/// | the root supervisor's join ([`crate::supervisor::SHUTDOWN_JOIN_DEADLINE`]) | 2s |
///
/// That is 29s of ~30s, and it does not count `agent.stop()` at the front.
/// **Anything added here has to come out of that margin** — check this table
/// before introducing another bounded step, and update it if you do.
///
/// The supervisor join is the single owner of the arithmetic at the tail: it
/// replaced a separate 1s join of the event outbox drainer, and it also absorbed
/// axum's `with_graceful_shutdown` wait, which `main` used to await *unbounded*
/// between the flushes and the drainer join.
///
/// On timeout the drainer keeps retrying in the background until the process
/// exits — best-effort, not a hard guarantee.
const OUTBOX_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Side effects to run when a shutdown signal arrives: stop the running agent
/// (best-effort), flush CLI session state to S3, then cancel the lifecycle
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

    // Drain the event outbox BEFORE the transcript flush and BEFORE cancelling
    // the lifecycle token (which stops the drainer). Stopping the agent above
    // emits the terminal `agent`/`taskTurn` events we most need delivered, so
    // this flush is what makes graceful teardown lose no lifecycle events.
    // Bounded so it can't blow the pod's 30s grace period — the transcript and
    // workspace flushes below already budget ~22s worst case.
    //
    // **This step's position is load-bearing, not just its duration.**
    // `EventOutbox::flush` early-returns the instant `drainer_stopped` is set —
    // deliberately, since waiting on a dead drainer is pure delay. So a
    // teardown that cancels the lifecycle token before (or concurrently with)
    // this line turns the flush into a no-op that returns instantly and reports
    // success: nothing fails, no test goes red, and the pod simply stops
    // delivering its terminal lifecycle events on every graceful teardown —
    // exactly what this flush exists to prevent. Pinned by
    // `tests/supervisor_test.rs::shutdown_flushes_the_outbox_before_cancelling`.
    if let Some(state) = state {
        state.events.outbox().flush(OUTBOX_FLUSH_TIMEOUT).await;
    }

    // Flush CLI session state AFTER the agent has stopped (so active JSONL
    // files are flushed by the CLI) and BEFORE cancelling the lifecycle token
    // (which stops the periodic sync). On a graceful kill — including a spot
    // reclaim's 2-min drain → SIGTERM — this persists the final delta with no
    // loss. Bounded internally so it can't blow the pod's 30s grace period.
    if let Some(state) = state {
        crate::init::transcript_sync::flush_on_shutdown(state).await;

        // For the `s3` storage backend, flush the git working tree too — this is
        // the durability the node-local emptyDir doesn't provide. Same ordering
        // rationale as the transcript flush; bounded internally.
        if state.config.workspace_sync_enabled {
            crate::init::workspace_sync::flush_on_shutdown(state).await;
        }
    }

    // Last, and only last. Every bounded step above depends on the task this
    // cancels still running — most sharply the outbox flush, see above.
    //
    // **This line is the single production origin of lifecycle cancellation**,
    // and that is what makes "cancel as an ordered step" safe rather than merely
    // convenient. `git grep 'lifecycle_cancel.cancel()' -- sandbox/src` returns
    // this site and nothing else in production code; the root supervisor does
    // not originate cancellation, it is *woken* by this line and then owns only
    // the bounded join that follows. On the fatal path the supervisor runs this
    // whole function before its own cancel, so the order is identical there.
    //
    // If you add a second origin, that argument stops holding: two sites racing
    // this one can cancel the drainer out from under the flush above and empty
    // it silently. Either keep this the only origin, or make the drainer's
    // cancellation explicitly ordered behind the flush.
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
///
/// Returns the task's join handle so the root supervisor owns its termination:
/// this task performs required cleanup, and losing it before the signal arrives
/// is what reintroduces the exit-137 kills this module exists to prevent. Its
/// own normal return happens only after [`perform_shutdown`] has cancelled the
/// lifecycle token, so the supervisor classifies it as shutdown, not failure.
#[must_use = "the shutdown driver owns required cleanup; register it with the supervisor"]
pub fn spawn_shutdown_driver(state: SandboxState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        await_shutdown_signal().await;
        info!("Shutdown signal received — stopping agent and winding down");
        let agent = Arc::clone(&state.agent);
        let lifecycle_cancel = state.lifecycle_cancel.clone();
        perform_shutdown(&agent, &lifecycle_cancel, Some(&state)).await;
    })
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

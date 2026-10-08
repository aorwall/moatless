//! Graceful shutdown handling for the sandbox runtime.
//!
//! The sandbox runs one agent per pod. Without signal handling the process
//! blocks on `axum::serve` and is SIGKILLed (exit 137) at the end of the pod's
//! termination grace period on *every* teardown — idle-down, node drain,
//! eviction, and PR-close. Installing a SIGTERM/SIGINT handler that stops the
//! agent, flushes CLI session state, and cancels the lifecycle token lets the
//! process drain and exit promptly instead.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::agent::agent_state::AgentState;
use crate::agent::types::AgentStatus;
use crate::state::SandboxState;

/// 5s budget for draining the event send buffer on shutdown — one of several
/// steps that must fit the pod's termination grace period
/// (`SANDBOX_TERMINATION_GRACE_SECS`).
///
/// On timeout the drainer keeps retrying in the background until the process
/// exits, best-effort only.
const OUTBOX_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// 2s budget for the writers admitted before teardown began to finish starting.
///
/// Overrunning it costs the pod its final-tree tag, not its flush, so this is
/// deliberately small against the two flushes below.
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// 3s budget for a cancelled agent's harness to wind down.
///
/// Same trade as the drain above: overrunning it costs the tag, since
/// `workspace_writers_are_quiet` reads the agent that is still running.
const AGENT_QUIESCE_TIMEOUT: Duration = Duration::from_secs(3);

/// How often the wait re-reads the agent's status, which has no change signal to
/// subscribe to.
const AGENT_QUIESCE_POLL: Duration = Duration::from_millis(25);

/// Stops the running agent (best-effort), flushes CLI session state to S3, then
/// cancels the lifecycle token so background tasks and both servers wind down.
///
/// `state` is `None` only to exercise agent-stop/token-cancel in isolation;
/// production always passes `Some`.
pub async fn perform_shutdown(
    agent: &AgentState,
    lifecycle_cancel: &CancellationToken,
    state: Option<&SandboxState>,
) {
    if let Some(state) = state {
        close_writer_admission(state).await;
    }
    stop_agent_and_wait(agent).await;
    finish_shutdown(lifecycle_cancel, state).await;
}

/// Refuse every new workspace writer, then wait out the admitted ones.
///
/// First, ahead of stopping any agent: a turn dispatched after that stop pass
/// is one nothing ends, and it writes past the flush that reads the tree.
async fn close_writer_admission(state: &SandboxState) {
    state
        .writer_admission
        .close_and_drain(WRITER_DRAIN_TIMEOUT)
        .await;
}

/// Variant used by the sandbox runtime, which owns all registered task contexts
/// and therefore stops every member agent on process teardown.
pub async fn perform_shutdown_for_state(state: &SandboxState) {
    close_writer_admission(state).await;
    let agents: Vec<Arc<AgentState>> = state
        .task_contexts_snapshot()
        .await
        .into_iter()
        .map(|context| Arc::clone(&context.agent))
        .collect();
    for agent in agents {
        stop_agent_and_wait(&agent).await;
    }
    finish_shutdown(&state.lifecycle_cancel, Some(state)).await;
}

/// Cancel the agent's turn, then wait for the harness to report it has wound
/// down.
///
/// `AgentState::stop` only cancels the token: the coordinator still has to end
/// the run and kill the harness, and a tree captured before that is a tree the
/// harness is still writing to.
async fn stop_agent_and_wait(agent: &AgentState) {
    stop_agent(agent).await;
    let quiesced = tokio::time::timeout(AGENT_QUIESCE_TIMEOUT, async {
        while matches!(
            agent.status().await,
            AgentStatus::Running | AgentStatus::Waiting
        ) {
            tokio::time::sleep(AGENT_QUIESCE_POLL).await;
        }
    })
    .await;
    if quiesced.is_err() {
        warn!("Shutdown: the agent was still running when its budget ran out");
    }
}

async fn stop_agent(agent: &AgentState) {
    // Cancel the in-flight turn so the Claude CLI subprocess is killed and the
    // turn ends now, instead of being SIGKILLed at grace-period end.
    // `AgentNotRunning` (an idle agent) is expected on a routine idle-down and
    // is ignored.
    //
    // This cancels via the token rather than an `AgentCommand::Stop`, and that
    // distinction is load-bearing: the coordinator reads the absent stop source
    // as "the pod is going away under a turn nobody asked to end" and closes the
    // turn as `turn_interrupted` instead of `stopped`. A `stopped` here would be
    // a terminal event, and `find_interrupted_turn` would tell the replacement
    // pod there is nothing to resume.
    match agent.stop().await {
        Ok(()) => info!("Shutdown: stopped running agent"),
        Err(e) => info!(error = %e, "Shutdown: no running agent to stop (continuing)"),
    }
}

async fn finish_shutdown(lifecycle_cancel: &CancellationToken, state: Option<&SandboxState>) {
    // Drain the event outbox BEFORE the transcript flush and BEFORE cancelling
    // the lifecycle token (which stops the drainer). Stopping the agent above
    // emits the terminal `agent`/`taskTurn` events we most need delivered, so
    // this flush is what makes graceful teardown lose no lifecycle events.
    // Bounded so it can't blow the pod's grace period; see
    // `every_shutdown_step_fits_the_grace_period`.
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

    // Both flushes run AFTER the agent has stopped (so the working tree is
    // stable and the CLI has flushed its active JSONL files) and BEFORE
    // cancelling the lifecycle token (which stops the periodic syncs). On a
    // graceful kill — including a spot reclaim's 2-min drain → SIGTERM — this
    // persists the final delta with no loss. Each is bounded internally so the
    // pair fits the pod's grace period.
    if let Some(state) = state {
        // The working tree goes first, deliberately: for the `s3` storage
        // backend it is the durability the node-local emptyDir doesn't provide,
        // and a tree lost here is lost outright, while a transcript the flush
        // below misses is still recoverable through `context_recovery_prompt`
        // reading the task's messages back.
        if state.config.workspace_sync_enabled {
            crate::workspace_sync::flush_on_shutdown(state).await;
        }

        crate::transcript_sync::flush_on_shutdown(state).await;
    }

    // Last, and only last. Every bounded step above depends on the task this
    // cancels still running — most sharply the outbox flush, see above.
    //
    // **This line is the single production origin of lifecycle cancellation**,
    // and that is what makes "cancel as an ordered step" safe rather than merely
    // convenient. `git grep 'lifecycle_cancel.cancel()' -- crates/sandbox/src` returns
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
/// Resolves as soon as either is delivered; SIGTERM is what Kubernetes sends on
/// pod termination, SIGINT covers `Ctrl-C` in local runs.
pub async fn await_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

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

/// Spawn the shutdown driver: wait for a signal, then run
/// [`perform_shutdown_for_state`]. Returns the join handle.
///
/// Losing this task before the signal reintroduces the exit-137 kill; returning
/// only after cancelling the lifecycle token reads as shutdown, not failure.
#[must_use = "the shutdown driver owns required cleanup; register it with the supervisor"]
pub fn spawn_shutdown_driver(state: SandboxState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        await_shutdown_signal().await;
        info!("Shutdown signal received — stopping agent and winding down");
        perform_shutdown_for_state(&state).await;
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// Kubernetes SIGKILLs the pod when its grace period ends, and a flush cut
    /// off there reports nothing. Each step is bounded on its own, so only their
    /// sum against the grace period shows they still fit after one is raised.
    #[test]
    fn every_shutdown_step_fits_the_grace_period() {
        let steps = [
            WRITER_DRAIN_TIMEOUT,
            // Per agent; a sandbox stops one agent per task it hosts.
            AGENT_QUIESCE_TIMEOUT,
            OUTBOX_FLUSH_TIMEOUT,
            crate::workspace_sync::SHUTDOWN_FLUSH_TIMEOUT,
            crate::workspace_sync::TIMEOUT_REPORT_TIMEOUT,
            crate::transcript_sync::SHUTDOWN_FLUSH_TIMEOUT,
            crate::supervisor::SHUTDOWN_JOIN_DEADLINE,
        ];
        let total: Duration = steps.iter().sum();
        let grace = Duration::from_secs(u64::from(
            moatless_types::sandbox::SANDBOX_TERMINATION_GRACE_SECS,
        ));

        assert!(
            total + Duration::from_secs(5) <= grace,
            "{total:?} of shutdown steps leaves under 5s of the {grace:?} grace period"
        );
    }

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

    /// Stopping the agents is one pass. A turn dispatched after it runs is one
    /// nothing ends, and it writes past the flush that reads the tree, so
    /// admission has to be closed before the first agent is stopped.
    #[tokio::test]
    async fn shutdown_closes_writer_admission_before_it_stops_an_agent() {
        let state = crate::state::test_support::state_for_test();
        let agent = Arc::new(AgentState::new());
        let agent_cancel = CancellationToken::new();
        agent.set_running(agent_cancel.clone()).await;

        let admission = Arc::clone(&state.writer_admission);
        let winding_down = Arc::clone(&agent);
        let when_stopped = tokio::spawn(async move {
            agent_cancel.cancelled().await;
            let closed = admission.is_closed();
            // Stands in for the coordinator, which is what ends the run.
            winding_down.set_stopped().await;
            closed
        });

        perform_shutdown(&agent, &CancellationToken::new(), Some(&state)).await;

        assert!(
            when_stopped.await.expect("join"),
            "the agent was stopped while a further turn could still be admitted"
        );
    }

    /// `AgentState::stop` only cancels the token. The harness winds down after
    /// that, and a capture taken in between reads a tree it is still writing.
    #[tokio::test]
    async fn shutdown_waits_for_the_stopped_agent_to_wind_down() {
        let agent = Arc::new(AgentState::new());
        let agent_cancel = CancellationToken::new();
        agent.set_running(agent_cancel.clone()).await;

        let wound_down = Arc::new(AtomicBool::new(false));
        let winding_down = Arc::clone(&agent);
        let reported = Arc::clone(&wound_down);
        tokio::spawn(async move {
            agent_cancel.cancelled().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            reported.store(true, Ordering::SeqCst);
            winding_down.set_stopped().await;
        });

        perform_shutdown(&agent, &CancellationToken::new(), None).await;

        assert!(
            wound_down.load(Ordering::SeqCst),
            "shutdown returned while the harness was still winding down"
        );
    }

    #[tokio::test]
    async fn perform_shutdown_stops_running_agent_and_cancels_token() {
        let agent = Arc::new(AgentState::new());
        let agent_cancel = CancellationToken::new();
        agent.set_running(agent_cancel.clone()).await;
        assert!(!agent_cancel.is_cancelled());

        let winding_down = Arc::clone(&agent);
        let cancelled = agent_cancel.clone();
        tokio::spawn(async move {
            cancelled.cancelled().await;
            winding_down.set_stopped().await;
        });

        let token = CancellationToken::new();
        perform_shutdown(&agent, &token, None).await;

        // The agent's in-flight turn was cancelled...
        assert!(agent_cancel.is_cancelled());
        // ...and the lifecycle token was cancelled.
        assert!(token.is_cancelled());
    }
}

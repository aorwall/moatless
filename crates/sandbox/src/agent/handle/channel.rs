use crate::agent::types::StopSource;
use tokio::sync::{mpsc, oneshot};

/// What the dispatcher needs to acknowledge a batch of messages: the CLI
/// session that consumed them and the agent run they belong to.
///
/// The run id is what lets the backend put messages injected mid-run on the
/// same turn as the run they joined, independently of whether the ack or the
/// run's `taskTurn.active` event reaches the backend first.
#[derive(Debug, Clone)]
pub struct RunAck {
    pub session_id: String,
    pub run_id: String,
}

/// Commands sent to the agent lifecycle owner.
pub(crate) enum AgentCommand {
    /// Send a message. The ack sender is resolved with the session and run IDs
    /// once the agent confirms it processed the message.
    Send {
        text: String,
        model: Option<String>,
        agent_mode: Option<String>,
        /// Reasoning effort for the turn, in the agent's own vocabulary.
        effort: Option<String>,
        /// Session ID from the backend for resuming after pod restart.
        resume_session_id: Option<String>,
        /// The user asked for a fresh agent session: never inject into a
        /// running one, and start the new run without `--resume`.
        start_fresh: bool,
        /// Where this Task's session branches from, on a fork's first turn.
        fork: Option<crate::agent::types::ForkSource>,
        ack: oneshot::Sender<RunAck>,
    },
    /// Replay an interrupted run on startup. Behaves identically to `Send`
    /// except the resume session ID is forced from the backend's recovery
    /// payload — the existing CLI `--resume <session_id>` path drives the
    /// rest. Acks are dropped (no caller waiting on a `session_id`).
    Recover {
        text: String,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: String,
        /// The interrupted turn's original `run_id`. Reused so the recovery run's
        /// events map back to that turn instead of forking a new one. `None`
        /// for a legacy turn with no recorded `run_id`.
        run_id: Option<String>,
    },
    /// Stop the agent. The `ack` (when present) resolves once the lifecycle
    /// owner has published the single terminal `Stopped` event and closed the
    /// Turn, so the caller learns the stop was processed rather than queued.
    /// `source` is echoed on the terminal event so the backend can attribute
    /// the stop (user vs. idle reaper vs. cleanup).
    Stop {
        source: StopSource,
        ack: Option<oneshot::Sender<()>>,
    },
}

/// Handle to the agent lifecycle owner — the only interface the outside world needs.
#[derive(Clone)]
pub struct AgentHandle {
    pub(in crate::agent) command_tx: mpsc::UnboundedSender<AgentCommand>,
}

impl AgentHandle {
    /// Send a message to the agent.
    ///
    /// `resume_session_id` is the backend-tracked session ID for resuming after
    /// pod restart. Pass `None` when injecting into an active Claude session or
    /// when no prior session is known.
    ///
    /// `start_fresh` forces a brand-new agent session: the message is never
    /// injected into a running one, and the new run starts without `--resume`.
    ///
    /// Returns a receiver that resolves with the session and run IDs once the
    /// agent has processed the message:
    /// - Claude injection: resolves immediately after a successful stdin write,
    ///   carrying the *running* run's id so the message joins its turn.
    /// - New Claude/Codex turn: resolves on `SessionInit` with the real session ID.
    #[allow(clippy::too_many_arguments)] // reason: one parameter per per-Turn field the command carries
    pub fn send(
        &self,
        text: String,
        model: Option<String>,
        _agent_type: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
        fork: Option<crate::agent::types::ForkSource>,
    ) -> oneshot::Receiver<RunAck> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            model,
            agent_mode,
            effort,
            resume_session_id,
            start_fresh,
            fork,
            ack: ack_tx,
        });
        ack_rx
    }

    /// Stop the agent. Returns a receiver that resolves once the lifecycle
    /// owner has published the single terminal `Stopped` event and closed the
    /// Turn — so a caller (e.g. the gRPC `stop_agent` handler) can wait for the
    /// stop to be *processed*, not merely queued.
    pub fn stop(&self, source: StopSource) -> oneshot::Receiver<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Stop {
            source,
            ack: Some(ack_tx),
        });
        ack_rx
    }

    /// Whether the lifecycle owner has exited (its command receiver was dropped).
    ///
    /// Used by the `/livez` liveness probe: a closed channel means no further
    /// agent turn can ever run, so the pod should be restarted.
    pub fn is_closed(&self) -> bool {
        self.command_tx.is_closed()
    }

    /// Replay an interrupted agent run on sandbox startup.
    ///
    /// `resume_session_id` is forced through to the CLI as `--resume <id>`,
    /// so the run picks up the on-disk Claude session JSONL state from the
    /// PVC. No ack — recovery is best-effort and the caller does not wait
    /// for a session id.
    pub fn recover(
        &self,
        text: String,
        _agent_type: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: String,
        run_id: Option<String>,
    ) {
        let _ = self.command_tx.send(AgentCommand::Recover {
            text,
            agent_mode,
            effort,
            resume_session_id,
            run_id,
        });
    }
}

#[cfg(test)]
impl AgentHandle {
    /// Build a handle backed by a fresh command channel, returning the receiver
    /// so a test can observe exactly which `AgentCommand`s the init/dispatch path
    /// enqueues. This channel is the single FIFO seam the lifecycle owner drains
    /// into the harness, so command order is the order the harness accepts input.
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<AgentCommand>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        (Self { command_tx }, command_rx)
    }
}

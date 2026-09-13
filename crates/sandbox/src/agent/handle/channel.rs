use crate::agent::types::StopSource;
use tokio::sync::{mpsc, oneshot};

/// What the dispatcher needs to acknowledge a batch of messages: session and
/// run ids.
///
/// The run id lets the backend attribute a mid-run injected message to its run,
/// whichever of the ack or `taskTurn.active` event arrives first.
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
    /// Replay an interrupted run on startup, behaving like `Send` but forcing
    /// the resume session ID from the backend's recovery payload. Acks are
    /// dropped: no caller waits on a `session_id`.
    Recover {
        text: String,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: String,
        /// The interrupted turn's original `run_id`. Reused so the recovery
        /// run's events map back to that turn instead of forking a new one.
        /// `None` for a legacy turn with no recorded `run_id`.
        run_id: Option<String>,
    },
    /// Stop the agent.
    ///
    /// `source` is echoed on the terminal `Stopped` event for attribution
    /// (user vs. idle reaper vs. cleanup). See [`AgentHandle::stop`] for what
    /// `ack` resolves.
    Stop {
        source: StopSource,
        ack: Option<oneshot::Sender<()>>,
    },
}

/// Handle to the agent lifecycle owner — the only interface the outside world
/// needs.
#[derive(Clone)]
pub struct AgentHandle {
    pub(in crate::agent) command_tx: mpsc::UnboundedSender<AgentCommand>,
}

impl AgentHandle {
    /// Send a message to the agent.
    ///
    /// The returned receiver resolves with the running run's id on injection
    /// or with `SessionInit`'s id on a new turn.
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

    /// Stop the agent.
    ///
    /// The returned receiver resolves only after the terminal `Stopped` event
    /// and Turn close, so a caller (e.g. gRPC `stop_agent`) can wait for the
    /// stop to be processed, not merely queued.
    pub fn stop(&self, source: StopSource) -> oneshot::Receiver<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Stop {
            source,
            ack: Some(ack_tx),
        });
        ack_rx
    }

    /// Whether the lifecycle owner has exited (its command receiver was
    /// dropped).
    ///
    /// Used by the `/livez` probe: a closed channel means no further agent turn
    /// can ever run, so the pod should be restarted.
    pub fn is_closed(&self) -> bool {
        self.command_tx.is_closed()
    }

    /// Replay an interrupted agent run on sandbox startup.
    ///
    /// `resume_session_id` is forced through as `--resume <id>` so the run
    /// picks up the on-disk Claude session JSONL state from the PVC; recovery
    /// is best-effort and takes no ack.
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
    /// Build a handle backed by a fresh command channel, for observing which
    /// `AgentCommand`s the init/dispatch path enqueues.
    ///
    /// This channel is the single FIFO seam the lifecycle owner drains into the
    /// harness, so command order is the order the harness accepts input.
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<AgentCommand>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        (Self { command_tx }, command_rx)
    }
}

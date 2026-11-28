use crate::agent::types::StopSource;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

/// Commands the lifecycle owner has yet to take, including every one sent
/// before the setup gate opens.
const COMMAND_CAPACITY: usize = 64;

/// What the dispatcher needs to acknowledge a batch of messages: session and
/// run ids.
///
/// The run id lets the backend attribute a mid-run injected message to its run,
/// whichever of the ack or `taskTurn.active` event arrives first.
/// A terminal configuration rejection settles the batch against its error Run.
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
        /// The Task's Agent profile; a different one than the running
        /// harness's rebuilds the harness before this turn.
        agent_profile: Option<String>,
        /// Open the Task's switch session into the profile's harness.
        switch_harness: bool,
        ack: oneshot::Sender<RunAck>,
    },
    /// Replay an interrupted run on startup, behaving like `Send` but forcing
    /// the resume session ID from the backend's recovery payload. Acks are
    /// dropped: no caller waits on a `session_id`.
    Recover {
        text: String,
        agent_mode: Option<String>,
        effort: Option<String>,
        model: Option<String>,
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
///
/// A `send` or `recover` made while [`COMMAND_CAPACITY`] are queued is
/// refused: its ack closes unresolved, which the dispatcher redelivers.
#[derive(Clone)]
pub struct AgentHandle {
    command_tx: mpsc::Sender<AgentCommand>,
}

impl AgentHandle {
    /// A handle and the receiver its lifecycle owner drains.
    pub(in crate::agent) fn channel() -> (Self, mpsc::Receiver<AgentCommand>) {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        (Self { command_tx }, command_rx)
    }

    fn submit(&self, command: AgentCommand) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.command_tx.try_send(command) {
            warn!(
                capacity = COMMAND_CAPACITY,
                "Agent command queue is full; refusing the command"
            );
        }
    }

    /// Send a message to the agent.
    ///
    /// The returned receiver resolves with the running run's id on injection
    /// or with `SessionInit`'s id on a new turn.
    #[expect(
        clippy::too_many_arguments,
        reason = "one parameter per per-Turn field the command carries"
    )]
    pub fn send(
        &self,
        text: String,
        model: Option<String>,
        agent_profile: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
        fork: Option<crate::agent::types::ForkSource>,
        switch_harness: bool,
    ) -> oneshot::Receiver<RunAck> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.submit(AgentCommand::Send {
            text,
            model,
            agent_mode,
            effort,
            resume_session_id,
            start_fresh,
            fork,
            agent_profile,
            switch_harness,
            ack: ack_tx,
        });
        ack_rx
    }

    /// Stop the agent, waiting for room in the queue rather than refusing.
    ///
    /// The returned receiver resolves only after the terminal `Stopped` event
    /// and Turn close, so a caller (e.g. gRPC `stop_agent`) can wait for the
    /// stop to be processed, not merely queued. It closes unresolved only when
    /// the lifecycle owner has exited.
    pub async fn stop(&self, source: StopSource) -> oneshot::Receiver<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = AgentCommand::Stop {
            source,
            ack: Some(ack_tx),
        };
        // A closed channel drops the command and its ack: the owner is gone.
        let _ = self.command_tx.send(command).await;
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
        agent_mode: Option<String>,
        effort: Option<String>,
        model: Option<String>,
        resume_session_id: String,
        run_id: Option<String>,
    ) {
        self.submit(AgentCommand::Recover {
            text,
            agent_mode,
            effort,
            model,
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
    pub(crate) fn test_channel() -> (Self, mpsc::Receiver<AgentCommand>) {
        Self::channel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_waits_for_room_in_a_full_queue() {
        let (handle, mut command_rx) = AgentHandle::channel();
        for _ in 0..COMMAND_CAPACITY {
            handle.recover(String::new(), None, None, None, String::new(), None);
        }
        let stopper = handle.clone();
        let stop = tokio::spawn(async move { stopper.stop(StopSource::User).await.await });
        tokio::task::yield_now().await;
        assert!(!stop.is_finished(), "stop must wait, not be refused");

        let mut acked = None;
        while let Some(command) = command_rx.recv().await {
            if let AgentCommand::Stop { ack, .. } = command {
                acked = ack.map(|ack| ack.send(()));
                break;
            }
        }
        assert_eq!(acked, Some(Ok(())));
        assert_eq!(stop.await.unwrap(), Ok(()));
    }
}

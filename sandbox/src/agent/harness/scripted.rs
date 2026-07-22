//! A deterministic scripted Agent process for lifecycle tests.
//!
//! This is the mock adapter role for the private [`AgentProcess`] port. It lets
//! contract tests drive Session starts, source blocks, activity, failures, EOF,
//! and cancellation through the real [`super::Harness`] interface without a CLI.
//! The interface under test is the external command/signal seam, not this type —
//! tests construct a [`Script`] and pass it to [`super::Harness::scripted`].

use super::error::HarnessError;
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Activity, Input, InputId, SessionId};
use super::source::SourceEvent;

/// One scripted step the process yields, in order, from [`AgentProcess::next`].
#[derive(Debug, Clone)]
pub enum Step {
    /// Report the process ready with a known Session identity.
    SessionReady(SessionId),
    /// Report acceptance of a unit of input.
    InputAccepted(InputId),
    /// Emit one native content block.
    Block(SourceEvent),
    /// Emit liveness / tool-progress activity.
    Activity(Activity),
    /// Terminate normally.
    Completed,
    /// Terminate with a classified failure.
    Failed(HarnessError),
    /// Close the process stream with no self-reported terminal (unexpected EOF).
    Eof,
    /// Block until the driver requests cancellation, then terminate. Models a
    /// long-running Turn interrupted by a cancel.
    WaitForCancel,
}

/// A scripted process definition: an optional submit failure plus an ordered
/// list of steps.
#[derive(Debug, Clone, Default)]
pub struct Script {
    /// When set, [`AgentProcess::submit`] fails with this error (a pre-start or
    /// input failure), and no steps run.
    pub submit_error: Option<HarnessError>,
    pub steps: Vec<Step>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            submit_error: None,
            steps,
        }
    }

    /// A script whose `submit` fails before any execution starts.
    pub fn rejecting(error: HarnessError) -> Self {
        Self {
            submit_error: Some(error),
            steps: Vec::new(),
        }
    }
}

pub(crate) struct ScriptedProcess {
    submit_error: Option<HarnessError>,
    steps: std::collections::VecDeque<Step>,
    cancelled: bool,
    cancel_notify: tokio::sync::Notify,
}

impl ScriptedProcess {
    pub(crate) fn new(script: Script) -> Self {
        Self {
            submit_error: script.submit_error,
            steps: script.steps.into(),
            cancelled: false,
            cancel_notify: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for ScriptedProcess {
    async fn submit(&mut self, _input: Input) -> Result<(), HarnessError> {
        match self.submit_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn cancel(&mut self) {
        self.cancelled = true;
        self.cancel_notify.notify_one();
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        // Peek so a `WaitForCancel` step is not lost if this future is dropped by
        // a concurrent `select!` branch and re-created — keeping `next` safe to
        // poll inside the driver's `select!`.
        if matches!(self.steps.front(), Some(Step::WaitForCancel)) {
            if !self.cancelled {
                self.cancel_notify.notified().await;
            }
            self.steps.pop_front();
            return Some(ProcessEvent::Terminated {
                termination: ProcessTermination::Completed,
            });
        }

        match self.steps.pop_front()? {
            Step::SessionReady(session) => Some(ProcessEvent::SessionReady { session }),
            Step::InputAccepted(input) => Some(ProcessEvent::InputAccepted { input }),
            Step::Block(block) => Some(ProcessEvent::Block(block)),
            Step::Activity(activity) => Some(ProcessEvent::Activity(activity)),
            Step::Completed => Some(ProcessEvent::Terminated {
                termination: ProcessTermination::Completed,
            }),
            Step::Failed(error) => Some(ProcessEvent::Terminated {
                termination: ProcessTermination::Failed(error),
            }),
            Step::Eof => None,
            // Handled by the peek above; unreachable via pop.
            Step::WaitForCancel => None,
        }
    }
}

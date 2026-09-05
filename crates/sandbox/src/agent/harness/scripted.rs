//! A deterministic scripted Agent process for lifecycle tests.
//!
//! This is the mock adapter role for the private [`AgentProcess`] port. It lets
//! contract tests drive Session starts, source blocks, failures, EOF,
//! and cancellation through the real [`super::Harness`] interface without a CLI.
//! The interface under test is the external command/signal seam, not this type —
//! tests construct a [`Script`] and pass it to [`super::Harness::scripted`].

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Input, InputId, SessionId};
use super::source::SourceEvent;

/// One scripted step the process yields, in order, from [`AgentProcess::next`].
// reason: mirrors `ProcessEvent`, which carries one canonical `SourceEvent` by
// value; boxing only the `Block` variant of this test scaffold would diverge from
// the type it scripts.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Step {
    /// Report the process ready with a known Session identity (an execution
    /// opened by the most recently submitted input).
    SessionReady(SessionId),
    /// Report a harness-internal execution starting with a known Session
    /// (persistent transports only).
    InternalStart(SessionId),
    /// Report acceptance of a unit of input.
    InputAccepted(InputId),
    /// Report acceptance of the oldest submitted-but-unaccepted input. Lets a
    /// persistent script accept whichever input the driver delivered without the
    /// script naming its id.
    AcceptInput,
    /// Block until another input is submitted, then continue. Models a
    /// persistent process idling between executions or mid-Turn injection.
    AwaitInput,
    /// Emit one native content block.
    Block(SourceEvent),
    /// Terminate normally.
    Completed,
    /// Terminate with a classified failure.
    Failed(HarnessError),
    /// End the current execution while the process persists (persistent
    /// transports only) — a Turn boundary, not process death.
    ExecutionEnded,
    /// Close the process stream with no self-reported terminal (unexpected EOF).
    Eof,
    /// Block until the driver requests cancellation, then terminate. Models a
    /// long-running Turn interrupted by a cancel.
    WaitForCancel,
    /// Block until the driver requests cancellation, then end the execution while
    /// the process persists (the persistent counterpart of [`Step::WaitForCancel`]).
    WaitForCancelThenEnd,
}

/// A scripted process definition: an optional submit failure plus an ordered
/// list of steps.
#[derive(Debug, Clone, Default)]
pub struct Script {
    /// When set, [`AgentProcess::submit`] fails with this error (a pre-start or
    /// input failure), and no steps run.
    pub submit_error: Option<HarnessError>,
    /// Whether the process reports [`AgentProcess::accepts_injection`], so the
    /// one-execution-per-process driver hands it mid-execution input instead of
    /// queueing that input for a later execution.
    pub injects: bool,
    /// When set, [`AgentProcess::submit`] parks until the cancel handle fires
    /// and then reports a failure, and no steps run. Models a transport whose
    /// opening handshake waits on the Agent — Codex awaits `initialize`,
    /// `thread/start` and `turn/start` — so a Stop has to reach it there.
    pub open_blocks: bool,
    pub steps: Vec<Step>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            submit_error: None,
            injects: false,
            open_blocks: false,
            steps,
        }
    }

    /// A script whose `submit` never returns on its own — it parks until the
    /// cancel handle fires. The only way past it is a Stop.
    #[cfg(test)]
    pub fn blocking_open() -> Self {
        Self {
            open_blocks: true,
            ..Self::new(Vec::new())
        }
    }

    /// A script whose process injects mid-execution input into the execution
    /// already running, the way the Claude subprocess adapter does.
    #[cfg(test)]
    pub fn injecting(steps: Vec<Step>) -> Self {
        Self {
            injects: true,
            ..Self::new(steps)
        }
    }

    /// A script whose `submit` fails before any execution starts.
    pub fn rejecting(error: HarnessError) -> Self {
        Self {
            submit_error: Some(error),
            injects: false,
            open_blocks: false,
            steps: Vec::new(),
        }
    }
}

/// A shared sink of the full `Input`s a scripted process accepted, in submit
/// order. Lets a lifecycle test assert exactly which `Input` (text + resume) the
/// harness received — proving harness acceptance, not just enqueue shape.
#[cfg(test)]
pub(crate) type InputRecorder = Arc<Mutex<Vec<Input>>>;

pub(crate) struct ScriptedProcess {
    submit_error: Option<HarnessError>,
    injects: bool,
    open_blocks: bool,
    steps: VecDeque<Step>,
    cancelled: bool,
    /// The handle the driver cancels through while `submit` holds the process.
    cancel_token: CancellationToken,
    cancel_notify: Notify,
    /// Submitted inputs not yet consumed by an `AcceptInput`, in submit order.
    inputs: VecDeque<InputId>,
    submit_notify: Notify,
    /// When set, every accepted `Input` is appended here (full payload).
    recorder: Option<Arc<Mutex<Vec<Input>>>>,
}

impl ScriptedProcess {
    pub(crate) fn new(script: Script) -> Self {
        Self {
            submit_error: script.submit_error,
            injects: script.injects,
            open_blocks: script.open_blocks,
            steps: script.steps.into(),
            cancelled: false,
            cancel_token: CancellationToken::new(),
            cancel_notify: Notify::new(),
            inputs: VecDeque::new(),
            submit_notify: Notify::new(),
            recorder: None,
        }
    }

    /// Like [`ScriptedProcess::new`] but records every accepted `Input` into a
    /// shared sink, so a test can observe what the harness received.
    #[cfg(test)]
    pub(crate) fn new_recording(script: Script, recorder: InputRecorder) -> Self {
        Self {
            recorder: Some(recorder),
            ..Self::new(script)
        }
    }
}

#[async_trait::async_trait]
impl AgentProcess for ScriptedProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        if let Some(error) = self.submit_error.take() {
            return Err(error);
        }
        if let Some(recorder) = &self.recorder {
            recorder.lock().await.push(input.clone());
        }
        self.inputs.push_back(input.id);
        self.submit_notify.notify_one();
        if self.open_blocks {
            // Record first, so a test can tell that the input reached the
            // process before it parked; then wait for the only thing that ends
            // this open. A real transport reports the teardown it suffered, and
            // the driver is responsible for calling that cancelled rather than
            // failed.
            self.cancel_token.cancelled().await;
            return Err(HarnessError::new(
                FailureStage::Startup,
                ErrorClass::ProcessExit,
                SessionImpact::Lost,
                "the scripted open was cancelled",
            ));
        }
        Ok(())
    }

    fn accepts_injection(&self) -> bool {
        self.injects
    }

    async fn cancel(&mut self) {
        self.cancelled = true;
        self.cancel_token.cancel();
        self.cancel_notify.notify_one();
    }

    fn cancel_handle(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            // Peek so a blocking step is not lost if this future is dropped by a
            // concurrent `select!` branch and re-created — keeping `next` safe to
            // poll inside the driver's `select!`.
            match self.steps.front() {
                Some(Step::WaitForCancel) => {
                    if !self.cancelled {
                        self.cancel_notify.notified().await;
                    }
                    self.steps.pop_front();
                    return Some(ProcessEvent::Terminated {
                        termination: ProcessTermination::Completed,
                    });
                }
                Some(Step::WaitForCancelThenEnd) => {
                    if !self.cancelled {
                        self.cancel_notify.notified().await;
                    }
                    self.steps.pop_front();
                    return Some(ProcessEvent::ExecutionEnded {
                        termination: ProcessTermination::Completed,
                    });
                }
                Some(Step::AwaitInput) => {
                    // Wait until an input is queued that no `AcceptInput` has yet
                    // consumed. The queue is authoritative; `Notify` only wakes the
                    // poll (and stores one permit, so a submit that arrives before
                    // we await is not lost).
                    while self.inputs.is_empty() {
                        self.submit_notify.notified().await;
                    }
                    self.steps.pop_front();
                    // The gate produces no event of its own; fall through to the
                    // next step.
                    continue;
                }
                _ => {}
            }

            return match self.steps.pop_front()? {
                Step::SessionReady(session) => Some(ProcessEvent::SessionReady {
                    session,
                    agent_turn_id: None,
                }),
                Step::InternalStart(session) => Some(ProcessEvent::InternalStart { session }),
                Step::InputAccepted(input) => Some(ProcessEvent::InputAccepted { input }),
                Step::AcceptInput => {
                    let input = self
                        .inputs
                        .pop_front()
                        .unwrap_or_else(|| InputId("scripted-accept-without-submit".to_string()));
                    Some(ProcessEvent::InputAccepted { input })
                }
                Step::Block(block) => Some(ProcessEvent::Block(block)),
                Step::Completed => Some(ProcessEvent::Terminated {
                    termination: ProcessTermination::Completed,
                }),
                Step::Failed(error) => Some(ProcessEvent::Terminated {
                    termination: ProcessTermination::Failed(error),
                }),
                Step::ExecutionEnded => Some(ProcessEvent::ExecutionEnded {
                    termination: ProcessTermination::Completed,
                }),
                Step::Eof => None,
                // Handled by the peek above; unreachable via pop.
                Step::WaitForCancel | Step::WaitForCancelThenEnd | Step::AwaitInput => None,
            };
        }
    }
}

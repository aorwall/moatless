//! The shared ordering state machine.
//!
//! The driver is harness-neutral: it consumes raw [`ProcessEvent`]s from an
//! [`AgentProcess`] and emits the external [`Signal`] contract, enforcing the
//! invariants that every harness must satisfy:
//!
//! - no source event is emitted before `ExecutionStarted` (early output is
//!   buffered and released in native order once the Session is known);
//! - every started execution emits exactly one terminal `ExecutionFinished`;
//! - a submitted input that never starts produces exactly one `InputRejected`;
//! - cancellation is idempotent and maps a terminated execution to a single
//!   cancelled outcome;
//! - nothing is silently dropped — bounded channels provide backpressure and a
//!   closed signal channel stops the driver instead of discarding output.

use std::collections::VecDeque;

use tokio::sync::mpsc;

use super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::signal::{Cause, Input, InputId, Outcome, Rejection, Signal};

/// Signalled when the coordinator dropped the signal receiver; the driver stops.
struct SignalClosed;

/// Run the driver loop until the command side closes and no work remains, or the
/// signal receiver is dropped.
///
/// `factory` mints a fresh [`AgentProcess`] for each execution — a one-shot
/// process per Turn for Codex-style harnesses, or (in later tickets) a handle to
/// a persistent process for Claude-style harnesses.
pub(crate) async fn run<P, F>(
    mut factory: F,
    mut submit_rx: mpsc::Receiver<Input>,
    mut cancel_rx: mpsc::Receiver<()>,
    signal_tx: mpsc::Sender<Signal>,
) where
    P: AgentProcess,
    F: FnMut() -> P + Send,
{
    let mut queue: VecDeque<Input> = VecDeque::new();
    let mut commands_open = true;
    let mut cancel_open = true;

    loop {
        let Some(input) = next_input(
            &mut queue,
            &mut submit_rx,
            &mut cancel_rx,
            &mut commands_open,
            &mut cancel_open,
        )
        .await
        else {
            return;
        };

        let mut process = factory();
        if run_execution(
            &mut process,
            input,
            &mut submit_rx,
            &mut cancel_rx,
            &signal_tx,
            &mut queue,
            &mut commands_open,
            &mut cancel_open,
        )
        .await
        .is_err()
        {
            // Signal receiver dropped — no consumer remains, so stop.
            return;
        }
    }
}

/// Obtain the next input that should open an execution: a queued input first,
/// otherwise the next submitted command. Cancellation while idle is a no-op.
/// Returns `None` when the command side is closed and nothing is queued.
async fn next_input(
    queue: &mut VecDeque<Input>,
    submit_rx: &mut mpsc::Receiver<Input>,
    cancel_rx: &mut mpsc::Receiver<()>,
    commands_open: &mut bool,
    cancel_open: &mut bool,
) -> Option<Input> {
    if let Some(input) = queue.pop_front() {
        return Some(input);
    }
    loop {
        if !*commands_open {
            return None;
        }
        tokio::select! {
            biased;
            cancelled = cancel_rx.recv(), if *cancel_open => {
                match cancelled {
                    // Cancel while idle is a no-op: loop and keep waiting.
                    Some(()) => {}
                    // Cancel side closed; disable the branch. A closed command
                    // side is the real stop signal.
                    None => *cancel_open = false,
                }
            }
            submitted = submit_rx.recv() => {
                if let Some(input) = submitted {
                    return Some(input);
                }
                *commands_open = false;
                return None;
            }
        }
    }
}

/// Drive one execution from opening input through its single terminal signal.
#[allow(clippy::too_many_arguments)] // reason: the ordering loop wires several channels; extracting a struct would only relocate the arity.
async fn run_execution<P: AgentProcess>(
    process: &mut P,
    input: Input,
    submit_rx: &mut mpsc::Receiver<Input>,
    cancel_rx: &mut mpsc::Receiver<()>,
    signal_tx: &mpsc::Sender<Signal>,
    queue: &mut VecDeque<Input>,
    commands_open: &mut bool,
    cancel_open: &mut bool,
) -> Result<(), SignalClosed> {
    let input_id = input.id.clone();

    // Deliver the opening input. A failure here is a pre-start failure.
    if let Err(error) = process.submit(input).await {
        return emit(
            signal_tx,
            Signal::InputRejected {
                input: input_id,
                reason: Rejection::Failed(error),
            },
        )
        .await;
    }

    let mut started = false;
    let mut cancel_requested = false;
    let mut early: Vec<super::source::SourceEvent> = Vec::new();

    loop {
        tokio::select! {
            biased;
            cancelled = cancel_rx.recv(), if *cancel_open && !cancel_requested => {
                match cancelled {
                    Some(()) => {
                        cancel_requested = true;
                        process.cancel().await;
                    }
                    None => *cancel_open = false,
                }
            }
            submitted = submit_rx.recv(), if *commands_open => {
                match submitted {
                    // Inputs arriving during an active execution are queued in
                    // order and open a later execution once this one terminates.
                    Some(next) => queue.push_back(next),
                    None => *commands_open = false,
                }
            }
            event = process.next() => {
                match event {
                    Some(ProcessEvent::SessionReady { session }) => {
                        if !started {
                            started = true;
                            emit(
                                signal_tx,
                                Signal::ExecutionStarted {
                                    session,
                                    cause: Cause::Input(input_id.clone()),
                                },
                            )
                            .await?;
                            for block in early.drain(..) {
                                emit(signal_tx, Signal::Source(block)).await?;
                            }
                        }
                    }
                    Some(ProcessEvent::InputAccepted { input }) => {
                        emit(signal_tx, Signal::InputAccepted { input }).await?;
                    }
                    Some(ProcessEvent::Block(block)) => {
                        if started {
                            emit(signal_tx, Signal::Source(block)).await?;
                        } else {
                            early.push(block);
                        }
                    }
                    Some(ProcessEvent::Activity(activity)) => {
                        emit(signal_tx, Signal::Activity(activity)).await?;
                    }
                    Some(ProcessEvent::Terminated { termination }) => {
                        return finalize(
                            signal_tx,
                            started,
                            cancel_requested,
                            Some(termination),
                            &input_id,
                        )
                        .await;
                    }
                    None => {
                        // EOF with no prior Terminated: an unexpected exit.
                        return finalize(signal_tx, started, cancel_requested, None, &input_id)
                            .await;
                    }
                }
            }
        }
    }
}

/// Emit the single terminal for an execution. `termination = None` means the
/// process stream reached EOF without a self-reported terminal.
async fn finalize(
    signal_tx: &mpsc::Sender<Signal>,
    started: bool,
    cancel_requested: bool,
    termination: Option<ProcessTermination>,
    input_id: &InputId,
) -> Result<(), SignalClosed> {
    if started {
        let outcome = if cancel_requested {
            Outcome::Cancelled
        } else {
            match termination {
                Some(ProcessTermination::Completed) => Outcome::Completed,
                Some(ProcessTermination::Failed(error)) => Outcome::Failed(error),
                None => Outcome::Failed(unexpected_eof(FailureStage::Execution)),
            }
        };
        emit(signal_tx, Signal::ExecutionFinished { outcome }).await
    } else {
        let reason = if cancel_requested {
            Rejection::Cancelled
        } else {
            match termination {
                Some(ProcessTermination::Failed(error)) => Rejection::Failed(error),
                Some(ProcessTermination::Completed) => Rejection::Failed(HarnessError::new(
                    FailureStage::Startup,
                    ErrorClass::ProcessExit,
                    SessionImpact::Unknown,
                    "process ended before execution start",
                )),
                None => Rejection::Failed(unexpected_eof(FailureStage::Startup)),
            }
        };
        emit(
            signal_tx,
            Signal::InputRejected {
                input: input_id.clone(),
                reason,
            },
        )
        .await
    }
}

fn unexpected_eof(stage: FailureStage) -> HarnessError {
    HarnessError::new(
        stage,
        ErrorClass::ProcessExit,
        SessionImpact::Unknown,
        "unexpected process EOF",
    )
}

/// Send one signal, propagating a closed signal channel as [`SignalClosed`] so
/// the driver stops rather than discarding output.
async fn emit(signal_tx: &mpsc::Sender<Signal>, signal: Signal) -> Result<(), SignalClosed> {
    signal_tx.send(signal).await.map_err(|_| SignalClosed)
}

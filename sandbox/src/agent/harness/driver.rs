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
use tracing::warn;

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

/// Run the persistent-process driver loop until the command side closes and the
/// process ends, or the signal receiver is dropped.
///
/// Unlike [`run`], the process is long-lived: it hosts many executions
/// (`ExecutionEnded` is a Turn boundary, not process death), accepts input
/// injected into an active execution, and may open harness-internal executions
/// on its own. The same ordering invariants hold — no `Source` before its
/// `ExecutionStarted`/`InputAccepted`, and exactly one terminal per execution —
/// enforced through the shared [`emit`]/[`flush_early`]/[`finalize`] helpers.
///
/// All persistent-transport policy (fingerprint compatibility, injection vs.
/// replacement, tmux teardown) lives inside the [`AgentProcess`]; this loop is
/// harness-neutral and only sequences the external contract.
pub(crate) async fn run_persistent<P: AgentProcess>(
    mut process: P,
    mut submit_rx: mpsc::Receiver<Input>,
    mut cancel_rx: mpsc::Receiver<()>,
    signal_tx: mpsc::Sender<Signal>,
) {
    let mut state = PersistentState::default();
    let mut commands_open = true;
    let mut cancel_open = true;

    loop {
        // Both command channels closed means the coordinator is gone: nothing
        // left to serve. Return and let the process drop (a persistent transport
        // tears down its Agent process on drop), rather than manufacturing a
        // terminal no one will read.
        if !commands_open && !cancel_open {
            return;
        }

        let woke = tokio::select! {
            biased;
            cancelled = cancel_rx.recv(), if cancel_open => match cancelled {
                Some(()) => Woke::Cancel,
                None => Woke::CancelClosed,
            },
            submitted = submit_rx.recv(), if commands_open => match submitted {
                Some(input) => Woke::Submit(input),
                None => Woke::SubmitClosed,
            },
            event = process.next() => Woke::Event(event),
        };

        match woke {
            Woke::Cancel => {
                // Cancel only bites during an active execution; while idle it is
                // a no-op. The process persists after cancelling the Turn.
                if state.active && !state.cancel_requested {
                    state.cancel_requested = true;
                    process.cancel().await;
                }
            }
            Woke::CancelClosed => cancel_open = false,
            Woke::Submit(input) => {
                let input_id = input.id.clone();
                match process.submit(input).await {
                    // Delivered: its acceptance (injection or a fresh start) will
                    // arrive as an event. Remember it so a fresh start can name
                    // its cause and each acceptance resolves in submit order.
                    Ok(()) => state.pending.push_back(input_id),
                    Err(error) => {
                        if emit(
                            &signal_tx,
                            Signal::InputRejected {
                                input: input_id,
                                reason: Rejection::Failed(error),
                            },
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            Woke::SubmitClosed => commands_open = false,
            Woke::Event(event) => {
                match drive_persistent_event(event, &mut state, &signal_tx).await {
                    Ok(true) => {}
                    // Process ended (Terminated / EOF) or the signal consumer is
                    // gone: the persistent driver is done.
                    Ok(false) | Err(SignalClosed) => return,
                }
            }
        }
    }
}

/// What woke the persistent loop. The channel receives are resolved inside the
/// `select!`; process interactions (`submit`, `cancel`) then run in the match arm
/// so the `process.next()` branch never overlaps another `&mut process` borrow.
enum Woke {
    Cancel,
    CancelClosed,
    Submit(Input),
    SubmitClosed,
    Event(Option<ProcessEvent>),
}

/// Per-execution ordering state for the persistent driver.
#[derive(Default)]
struct PersistentState {
    /// An execution has started and not yet reached its terminal.
    active: bool,
    /// The active execution's opening input has been accepted (or it is a
    /// harness-internal execution) — the gate that releases buffered `Source`s.
    accepted: bool,
    /// A `SessionReady` opened the active execution and its `InputAccepted` has
    /// not arrived yet, so the next acceptance opens rather than injects.
    awaiting_open_accept: bool,
    /// Cancellation was requested for the active execution.
    cancel_requested: bool,
    /// Inputs delivered to the process whose acceptance has not been seen, in
    /// submit order. The front names the input that a fresh start opens.
    pending: VecDeque<InputId>,
    /// Source events held until the active execution has started and accepted.
    early: Vec<super::source::SourceEvent>,
}

/// Apply one raw persistent-process event to the external contract. Returns
/// `Ok(true)` to keep driving, `Ok(false)` when the process itself has ended.
async fn drive_persistent_event(
    event: Option<ProcessEvent>,
    state: &mut PersistentState,
    signal_tx: &mpsc::Sender<Signal>,
) -> Result<bool, SignalClosed> {
    match event {
        Some(ProcessEvent::SessionReady { session }) => {
            state.active = true;
            state.accepted = false;
            state.awaiting_open_accept = true;
            state.cancel_requested = false;
            let cause = Cause::Input(
                state
                    .pending
                    .front()
                    .cloned()
                    .unwrap_or_else(|| InputId(String::new())),
            );
            emit(signal_tx, Signal::ExecutionStarted { session, cause }).await?;
            Ok(true)
        }
        Some(ProcessEvent::InternalStart { session }) => {
            // No pending input behind an internal execution: sources release as
            // soon as it starts, and no `InputAccepted` follows.
            state.active = true;
            state.accepted = true;
            state.awaiting_open_accept = false;
            state.cancel_requested = false;
            emit(
                signal_tx,
                Signal::ExecutionStarted {
                    session,
                    cause: Cause::Internal,
                },
            )
            .await?;
            flush_early(signal_tx, state.active, state.accepted, &mut state.early).await?;
            Ok(true)
        }
        Some(ProcessEvent::InputAccepted { input }) => {
            // Consume this input from the pending set in submit order.
            if let Some(pos) = state.pending.iter().position(|id| id == &input) {
                state.pending.remove(pos);
            }
            if state.awaiting_open_accept {
                // Acceptance that opens the current execution.
                state.awaiting_open_accept = false;
                state.accepted = true;
                emit(signal_tx, Signal::InputAccepted { input }).await?;
                flush_early(signal_tx, state.active, state.accepted, &mut state.early).await?;
            } else {
                // Injection into an already-open execution: no new start.
                emit(signal_tx, Signal::InputAccepted { input }).await?;
            }
            Ok(true)
        }
        Some(ProcessEvent::Block(block)) => {
            if state.active && state.accepted {
                emit(signal_tx, Signal::Source(block)).await?;
            } else {
                state.early.push(block);
            }
            Ok(true)
        }
        Some(ProcessEvent::ContextWindow(window)) => {
            emit(signal_tx, Signal::ContextWindow(window)).await?;
            Ok(true)
        }
        Some(ProcessEvent::Activity(activity)) => {
            emit(signal_tx, Signal::Activity(activity)).await?;
            Ok(true)
        }
        Some(ProcessEvent::ExecutionEnded { termination }) => {
            // Terminal for the current execution; the process persists. Release
            // buffered source only if the input was actually accepted — a terminal
            // never synthesizes acceptance, so unanchored blocks are dropped.
            flush_early(signal_tx, state.active, state.accepted, &mut state.early).await?;
            state.early.clear();
            emit_terminal(signal_tx, state.cancel_requested, Some(termination)).await?;
            state.active = false;
            state.accepted = false;
            state.awaiting_open_accept = false;
            state.cancel_requested = false;
            Ok(true)
        }
        Some(ProcessEvent::Terminated { termination }) => {
            flush_early(signal_tx, state.active, state.accepted, &mut state.early).await?;
            if state.active {
                emit_terminal(signal_tx, state.cancel_requested, Some(termination)).await?;
            }
            Ok(false)
        }
        None => {
            flush_early(signal_tx, state.active, state.accepted, &mut state.early).await?;
            if state.active {
                // EOF mid-execution: an unexpected exit.
                emit_terminal(signal_tx, state.cancel_requested, None).await?;
            }
            // EOF while idle is a clean shutdown of a persistent process.
            Ok(false)
        }
    }
}

/// Emit the single `ExecutionFinished` for a persistent execution that has
/// started. A requested cancel maps any termination to `Cancelled`.
async fn emit_terminal(
    signal_tx: &mpsc::Sender<Signal>,
    cancel_requested: bool,
    termination: Option<ProcessTermination>,
) -> Result<(), SignalClosed> {
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
    let mut started = false;
    let mut accepted = false;
    let mut cancel_requested = false;

    // Deliver the opening input, racing cancellation against it. The open is not
    // instant for every transport — Codex awaits `initialize`, `thread/start`
    // and `turn/start` before `submit` returns — and until it does, no execution
    // exists for the `select!` below to cancel. Without this race a Stop issued
    // during the open would sit unread until the Agent answered, which is
    // exactly when it cannot.
    //
    // `cancel` is unreachable here (the `submit` future borrows the process), so
    // the interrupt goes through the process's own handle.
    let cancel_handle = process.cancel_handle();
    let submitted = {
        let mut submit = std::pin::pin!(process.submit(input));
        loop {
            tokio::select! {
                biased;
                cancelled = cancel_rx.recv(), if *cancel_open && !cancel_requested => {
                    match cancelled {
                        Some(()) => {
                            cancel_requested = true;
                            cancel_handle.cancel();
                        }
                        None => *cancel_open = false,
                    }
                }
                result = &mut submit => break result,
            }
        }
    };

    if let Err(error) = submitted {
        // A cancelled open reports as cancelled rather than failed: the error is
        // the teardown this driver asked for, not a fault of the Agent's.
        let reason = if cancel_requested {
            Rejection::Cancelled
        } else {
            Rejection::Failed(error)
        };
        return emit(
            signal_tx,
            Signal::InputRejected {
                input: input_id,
                reason,
            },
        )
        .await;
    }

    // The open won the race. The execution is live, so tear it down through the
    // normal path — its terminal is `Cancelled` because `cancel_requested` is
    // already set — rather than rejecting an input the process did accept.
    if cancel_requested {
        process.cancel().await;
    }
    // Input acceptance seen before `ExecutionStarted` — held so the terminal
    // order stays `ExecutionStarted → InputAccepted → Source*`.
    let mut pending_accept: Option<InputId> = None;
    // Source events held until both the execution has started and its input has
    // been accepted, so no `Source` is ever emitted before `InputAccepted`.
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
                    // A process that injects takes the input into the running
                    // execution, which reports it as a bare `InputAccepted` and
                    // so puts it on this execution's Turn. Otherwise — and if an
                    // injection fails — the input is queued in order and opens a
                    // later execution once this one terminates, so it is never
                    // dropped.
                    //
                    // Never inject into an execution that is already unwinding:
                    // its terminal is `Cancelled` whatever the process reports,
                    // so an accepted injection would be acknowledged against a
                    // Turn that produces nothing and would never open the
                    // follow-up execution the input was meant for.
                    Some(next) => {
                        if !cancel_requested && process.accepts_injection() {
                            // Raced against cancellation for the same reason the
                            // open above is: an injecting process can wait inside
                            // `submit` (Codex waits out its steerability gate),
                            // and this loop is the only reader of `cancel_rx`, so
                            // an unraced await would make a Stop unreadable for
                            // as long as that wait lasted. `cancel` is
                            // unreachable while the `submit` future borrows the
                            // process, so the interrupt goes through the handle.
                            let cancel_handle = process.cancel_handle();
                            let injected = {
                                let mut submit = std::pin::pin!(process.submit(next.clone()));
                                loop {
                                    tokio::select! {
                                        biased;
                                        cancelled = cancel_rx.recv(), if *cancel_open && !cancel_requested => {
                                            match cancelled {
                                                Some(()) => {
                                                    cancel_requested = true;
                                                    cancel_handle.cancel();
                                                }
                                                None => *cancel_open = false,
                                            }
                                        }
                                        result = &mut submit => break result,
                                    }
                                }
                            };
                            if let Err(error) = injected {
                                warn!(
                                    input = %next.id.0,
                                    %error,
                                    "injection into the active execution failed; queueing for a later one"
                                );
                                queue.push_back(next);
                            }
                            // The Stop won the race. Tear the execution down
                            // through the normal path, as the open does: its
                            // terminal is `Cancelled` because `cancel_requested`
                            // is already set.
                            if cancel_requested {
                                process.cancel().await;
                            }
                        } else {
                            queue.push_back(next);
                        }
                    }
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
                            // Release acceptance (then buffered sources) that
                            // arrived before the start, in contract order.
                            if let Some(input) = pending_accept.take() {
                                accepted = true;
                                emit(signal_tx, Signal::InputAccepted { input }).await?;
                            }
                            flush_early(signal_tx, started, accepted, &mut early).await?;
                        }
                    }
                    Some(ProcessEvent::InputAccepted { input }) => {
                        if started {
                            accepted = true;
                            emit(signal_tx, Signal::InputAccepted { input }).await?;
                            flush_early(signal_tx, started, accepted, &mut early).await?;
                        } else {
                            // Acceptance before start: hold it until the start is
                            // emitted so it never precedes `ExecutionStarted`.
                            pending_accept = Some(input);
                        }
                    }
                    Some(ProcessEvent::Block(block)) => {
                        if started && accepted {
                            emit(signal_tx, Signal::Source(block)).await?;
                        } else {
                            early.push(block);
                        }
                    }
                    Some(ProcessEvent::ContextWindow(window)) => {
                        emit(signal_tx, Signal::ContextWindow(window)).await?;
                    }
                    Some(ProcessEvent::Activity(activity)) => {
                        emit(signal_tx, Signal::Activity(activity)).await?;
                    }
                    // A one-shot execution ends by the process ending. An
                    // `ExecutionEnded` (persistent Turn boundary) or `InternalStart`
                    // are not part of the one-shot lifecycle; the former is treated
                    // as the terminal, the latter is ignored.
                    Some(ProcessEvent::ExecutionEnded { termination })
                    | Some(ProcessEvent::Terminated { termination }) => {
                        // Release buffered source only if acceptance was actually
                        // emitted; a terminal never synthesizes it.
                        flush_early(signal_tx, started, accepted, &mut early).await?;
                        return finalize(
                            signal_tx,
                            started,
                            cancel_requested,
                            Some(termination),
                            &input_id,
                        )
                        .await;
                    }
                    Some(ProcessEvent::InternalStart { .. }) => {}
                    None => {
                        // EOF with no prior Terminated: an unexpected exit.
                        flush_early(signal_tx, started, accepted, &mut early).await?;
                        return finalize(signal_tx, started, cancel_requested, None, &input_id)
                            .await;
                    }
                }
            }
        }
    }
}

/// Release buffered source events once the execution has both started and had
/// its input accepted. A no-op until then, so `Source` never precedes
/// `InputAccepted` — including at a terminal, which never synthesizes acceptance:
/// buffered blocks with no real `InputAccepted` are dropped rather than emitted
/// out of contract order.
async fn flush_early(
    signal_tx: &mpsc::Sender<Signal>,
    started: bool,
    accepted: bool,
    early: &mut Vec<super::source::SourceEvent>,
) -> Result<(), SignalClosed> {
    if !(started && accepted) {
        return Ok(());
    }
    for block in early.drain(..) {
        emit(signal_tx, Signal::Source(block)).await?;
    }
    Ok(())
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

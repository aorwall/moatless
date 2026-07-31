//! The Agent harness — a deep module with a small command/signal interface.
//!
//! A [`Harness`] controls at most one active Agent process while letting the
//! Session survive process replacement and Sandbox recreation. Callers submit
//! input, cancel the active execution, and receive an ordered stream of
//! normalized [`Signal`]s. They never receive a Claude or Codex handle and never
//! match on the selected harness — the implementation is fixed at construction
//! (ADR-0004, ADR-0005).
//!
//! What the harness owns: Agent process lifecycle, Session start/resume, input
//! delivery, cancellation, native normalization into canonical [`SourceEvent`]s,
//! and error classification. What it does **not** own: Run identity, Turn state,
//! pending-Message acknowledgement, retry policy, credential policy, backend
//! publishing, or Message assembly — all of that is the coordinator's.
//!
//! Module shape:
//! - [`signal`] — the external command/signal vocabulary
//! - [`source`] — canonical one-block source events
//! - [`error`] — stable harness error facts
//! - `process` — the private Agent-process port (production + mock adapters)
//! - `driver` — the shared ordering state machine
//! - `scripted` — the scripted mock process for lifecycle tests

pub mod error;
pub mod signal;
pub mod source;

mod claude;
mod codex;
mod driver;
mod process;

// The scripted mock and its `Harness::scripted` constructor exist only to drive
// the lifecycle/contract tests, so they stay behind `cfg(test)` — never part of
// the crate's runtime surface.
#[cfg(test)]
mod scripted;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod persistent_tests;

use tokio::sync::mpsc;

pub use claude::ClaudeConfig;
pub use codex::CodexConfig;
pub use error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
#[cfg(test)]
pub(crate) use scripted::{Script, Step};
pub use signal::{Activity, Cause, Input, InputId, Outcome, Rejection, SessionId, Signal};
pub use source::{NativeMessageId, Role, SourceEvent};

/// Bounded capacity for submitted inputs awaiting an execution slot. Full means
/// `submit` awaits capacity (explicit backpressure), never a silent drop.
const SUBMIT_CAPACITY: usize = 16;
/// Bounded capacity for the normalized signal stream.
const SIGNAL_CAPACITY: usize = 64;

/// The signal channel closed because the harness driver stopped (its command
/// side was dropped, or it observed a closed signal consumer).
#[derive(Debug, thiserror::Error)]
#[error("agent harness is closed")]
pub struct HarnessClosed;

/// Supervise a spawned driver task so its termination is never silent: an
/// abnormal end (a panic in the ordering state machine) is logged at `error`
/// with a diagnostic, a clean stop at `debug`. Without this a panicked driver
/// would collapse the harness channels — every `submit`/`next_signal` failing —
/// with no trace of why (Sandbox rule: handle spawned-task success and failure).
fn supervise(handle: tokio::task::JoinHandle<()>) {
    tokio::spawn(async move {
        match handle.await {
            Ok(()) => tracing::debug!("agent harness driver task ended"),
            Err(error) => {
                tracing::error!(%error, "agent harness driver task terminated abnormally");
            }
        }
    });
}

/// The deep Agent-harness module. Construction fixes the implementation for the
/// Sandbox lifetime; the type is concrete, so there is no public harness trait.
pub struct Harness {
    submit_tx: mpsc::Sender<Input>,
    cancel_tx: mpsc::Sender<()>,
    signal_rx: mpsc::Receiver<Signal>,
}

impl Harness {
    /// Construct a harness driven by a scripted mock process. The first
    /// execution runs `script`; any later execution ends immediately at EOF.
    #[cfg(test)]
    pub(crate) fn scripted(script: Script) -> Self {
        Self::scripted_with_signal_capacity(script, SIGNAL_CAPACITY)
    }

    /// Like [`Harness::scripted`] but with an explicit signal-channel capacity,
    /// for exercising backpressure.
    #[cfg(test)]
    pub(crate) fn scripted_with_signal_capacity(script: Script, signal_capacity: usize) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(signal_capacity.max(1));

        let mut first = Some(script);
        let factory = move || scripted::ScriptedProcess::new(first.take().unwrap_or_default());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Like [`Harness::scripted`] but every execution records the full `Input` it
    /// accepts into the shared `recorder`, so a lifecycle test can prove which
    /// input the harness received (text + resume), not merely that a command was
    /// enqueued. The same `script` runs for each execution.
    #[cfg(test)]
    pub(crate) fn scripted_recording(script: Script, recorder: scripted::InputRecorder) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let factory =
            move || scripted::ScriptedProcess::new_recording(script.clone(), recorder.clone());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct a one-shot harness whose executions run `scripts` in order — one
    /// script per execution, so a test can model a first execution that rejects
    /// (e.g. a failed resume) followed by a second that succeeds (the recovery
    /// re-drive). Executions past the list end immediately at EOF.
    #[cfg(test)]
    pub(crate) fn scripted_sequence(scripts: Vec<Script>) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let mut scripts = std::collections::VecDeque::from(scripts);
        let factory =
            move || scripted::ScriptedProcess::new(scripts.pop_front().unwrap_or_default());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct a harness driven by a single scripted *persistent* process: one
    /// long-lived process that hosts many executions, so the script can model
    /// injection, harness-internal executions, and Turn boundaries.
    #[cfg(test)]
    pub(crate) fn scripted_persistent(script: Script) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let process = scripted::ScriptedProcess::new(script);
        supervise(tokio::spawn(driver::run_persistent(
            process, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct the production Claude subprocess harness. Each execution launches
    /// a fresh one-shot CLI subprocess (the persistent tmux transport is a later
    /// ticket); the Turn-independent `config` is cloned per execution.
    pub fn claude(config: ClaudeConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let factory = move || claude::ClaudeProcess::new(config.clone());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct the production Codex harness. Codex is one-process-per-Turn, so
    /// each execution launches a fresh `codex app-server --stdio` subprocess
    /// through the shared one-shot [`driver::run`]; a later Turn resumes the
    /// known thread by replacing the process. The Turn-independent `config` is
    /// cloned per execution.
    pub fn codex(config: CodexConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let factory = move || codex::CodexProcess::new(config.clone());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct the production persistent Claude harness. One long-lived tmux
    /// TUI session hosts every Turn: compatible input is pasted into the live
    /// session (injected into an active Turn or opening a fresh one), and an
    /// incompatible model / mode / Session / fresh-start replaces the process
    /// while resuming the intended Session.
    pub fn claude_persistent(config: ClaudeConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);

        let process = claude::ClaudePersistentProcess::new(config);
        supervise(tokio::spawn(driver::run_persistent(
            process, submit_rx, cancel_rx, signal_tx,
        )));

        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Submit input. Awaits channel capacity (explicit backpressure); errors only
    /// when the harness has stopped.
    pub async fn submit(&self, input: Input) -> Result<(), HarnessClosed> {
        self.submit_tx.send(input).await.map_err(|_| HarnessClosed)
    }

    /// Cancel the active execution. Idempotent: a no-op while idle, and a
    /// second request while a cancel is already pending is coalesced (the
    /// capacity-1 channel already holds the request), never queued twice.
    pub fn cancel(&self) {
        let _ = self.cancel_tx.try_send(());
    }

    /// Receive the next normalized signal, or `None` once the harness has
    /// stopped and drained.
    pub async fn next_signal(&mut self) -> Option<Signal> {
        self.signal_rx.recv().await
    }
}

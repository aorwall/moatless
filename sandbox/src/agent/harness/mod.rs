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
mod driver;
mod process;

// The scripted mock and its `Harness::scripted` constructor exist only to drive
// the lifecycle/contract tests, so they stay behind `cfg(test)` — never part of
// the crate's runtime surface.
#[cfg(test)]
mod scripted;

#[cfg(test)]
mod contract_tests;

use tokio::sync::mpsc;

pub use claude::ClaudeConfig;
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
        tokio::spawn(driver::run(factory, submit_rx, cancel_rx, signal_tx));

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
        tokio::spawn(driver::run(factory, submit_rx, cancel_rx, signal_tx));

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

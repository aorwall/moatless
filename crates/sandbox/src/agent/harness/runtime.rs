use super::{ClaudeConfig, CodexConfig, Input, Signal, claude, codex, driver};
#[cfg(test)]
use super::{Script, scripted};
use tokio::sync::mpsc;

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
/// abnormal end (a panic) is logged at `error`, a clean stop at `debug`.
///
/// Without this a panicked driver would collapse the harness channels — every
/// `submit`/`next_signal` failing — with no trace of why.
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
pub struct HarnessRuntime {
    submit_tx: mpsc::Sender<Input>,
    cancel_tx: mpsc::Sender<()>,
    signal_rx: mpsc::Receiver<Signal>,
}

impl HarnessRuntime {
    /// Construct a mini-swe-agent bridge process for each execution.
    pub fn mini_swe(config: super::MiniSweConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);
        let factory = move || super::mini_swe::MiniSweProcess::new(config.clone());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));
        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct the persistent ACP harness: one agent process hosts every
    /// Turn, and outlives them so a background command can wake it.
    pub fn acp(config: super::AcpConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);
        let process = super::acp::AcpProcess::new(config);
        supervise(tokio::spawn(driver::run_persistent(
            process, submit_rx, cancel_rx, signal_tx,
        )));
        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct a Pi RPC process for each execution.
    pub fn pi(config: super::PiConfig) -> Self {
        let (submit_tx, submit_rx) = mpsc::channel(SUBMIT_CAPACITY);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        let (signal_tx, signal_rx) = mpsc::channel(SIGNAL_CAPACITY);
        let factory = move || super::pi::PiProcess::new(config.clone());
        supervise(tokio::spawn(driver::run(
            factory, submit_rx, cancel_rx, signal_tx,
        )));
        Self {
            submit_tx,
            cancel_tx,
            signal_rx,
        }
    }

    /// Construct a harness driven by a scripted mock process. The first
    /// execution runs `script`; any later execution ends immediately at EOF.
    #[cfg(test)]
    pub(crate) fn scripted(script: Script) -> Self {
        Self::scripted_with_signal_capacity(script, SIGNAL_CAPACITY)
    }

    /// Like [`HarnessRuntime::scripted`] but with an explicit signal-channel
    /// capacity, for exercising backpressure.
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

    /// Like [`HarnessRuntime::scripted`] but every execution records the full
    /// `Input` it accepts into `recorder`, so a lifecycle test proves which
    /// input (text + resume) the harness received. Each execution runs `script`.
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

    /// Construct a one-shot harness whose executions run `scripts` in order —
    /// one script per execution.
    ///
    /// Lets a test model a first execution that rejects (e.g. a failed resume)
    /// followed by a second that succeeds (recovery re-drive); executions past
    /// the list end immediately at EOF.
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

    /// Construct a harness driven by a single scripted *persistent* process:
    /// one long-lived process that hosts many executions, so the script can
    /// model injection, harness-internal executions, and Turn boundaries.
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

    /// Construct the production Claude subprocess harness. Each execution
    /// launches a fresh one-shot CLI subprocess (the persistent tmux transport
    /// is a later ticket); the Turn-independent `config` is cloned per
    /// execution.
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

    /// Construct the production Codex harness.
    ///
    /// Codex is one-process-per-Turn: each execution launches a fresh `codex
    /// app-server --stdio` subprocess through the shared one-shot
    /// [`driver::run`], and a later Turn resumes the known thread by replacing
    /// the process; the Turn-independent `config` is cloned per execution.
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

    /// Construct the production persistent Claude harness.
    ///
    /// One long-lived tmux TUI session hosts every Turn: compatible input
    /// pastes into the live session (injecting into an active Turn or opening a
    /// fresh one), and an incompatible model/mode/Session/fresh-start replaces
    /// the process while resuming the intended Session.
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

    /// Submit input. Awaits channel capacity (explicit backpressure); errors
    /// only when the harness has stopped.
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

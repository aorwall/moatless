//! Tmux-backed query mode.
//!
//! Runs the real interactive Claude TUI inside a detached tmux session and
//! exposes the same `QueryHandle` shape as the subprocess backend. Use this
//! when you need the full TUI experience (skills, slash commands, plugins
//! that don't fire in `--print` mode).
//!
//! Lifecycle is hook-driven: a per-run [`hooks::HookHarness`] writes inline
//! `--settings` JSON registering `SessionStart` + `Stop` hooks that point at
//! a relay script. The script appends `"<event>\t<payload>\n"` lines to a
//! FIFO; a blocking-thread reader forwards parsed lines into a watch channel
//! consumed by both this module (SessionStart wait, transcript discovery)
//! and the supervisor (Stop fallback for the Result emission).
//!
//! Trade-offs vs the subprocess backend:
//! - Permissions are always bypassed (`--dangerously-skip-permissions`).
//!   `ClaudeOptions.can_use_tool` is ignored.
//! - SDK MCP servers (`sdk_mcp_servers`) are not supported — the TUI owns its
//!   own MCP lifecycle.
//! - No streaming partials. Messages arrive once the TUI flushes complete
//!   blocks to the JSONL transcript.
//! - `total_cost_usd` is always `0.0` (synthesized; the TUI doesn't expose
//!   real cost data).
//!
//! Startup dialogs: before SessionStart fires, the TUI can block on (1) the
//! workspace-trust prompt ("Do you trust the files in this folder?") and (2)
//! the bypass-permissions warning shown by `--dangerously-skip-permissions`.
//! [`startup_dialog_watcher`] polls `tmux capture-pane` every 250ms during the
//! pre-SessionStart window: it sends Enter to accept trust (default selection),
//! and Down+Enter to accept the bypass dialog (whose default is "No, exit").

mod args;
mod hooks;
mod session;
mod subagents;
mod tailer;
mod translator;
mod trust;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::{Result, SdkError};
use crate::query::QueryHandle;
use crate::transport::StdinWriter;
use crate::types::messages::SdkMessage;
use crate::types::options::ClaudeOptions;

pub use self::session::has_resumable_transcript;

use self::hooks::{spawn_reader, HookHarness, HookLine};
use self::session::{list_jsonl_stems, resolve_project_dir, resolve_resume_target, TmuxSession};
use self::tailer::{
    discover_transcript_path, spawn_hook_listener, spawn_lazy_supervisor, spawn_supervisor,
    synth_system_init, HookState, TailTarget,
};
use self::trust::{scan_for_bypass_dialog, scan_for_trust_dialog};
use self::writer::TmuxPasteWriter;

/// Channel capacity for the message stream. Matches the subprocess path.
const CHANNEL_CAPACITY: usize = 256;

/// Default upper bound on waiting for the `SessionStart` hook to fire.
/// Overridden by `ClaudeOptions.init_timeout`.
const DEFAULT_SESSION_START_TIMEOUT: Duration = Duration::from_mins(1);

/// How often the trust-dialog watcher captures the tmux pane while waiting
/// for SessionStart. Only active in the brief pre-SessionStart window; auto-
/// stops once either SessionStart fires or the dialog is dismissed.
const TRUST_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// One-shot tmux query. Spawns the TUI, pastes `prompt` as the first user
/// message, and returns a `QueryHandle` for consuming the response stream.
pub async fn query_tmux(prompt: &str, mut options: ClaudeOptions) -> Result<QueryHandle> {
    warn_on_ignored_options(&options);

    let project_dir = resolve_project_dir(&options)?;
    tokio::fs::create_dir_all(&project_dir).await.map_err(|e| {
        crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        ))
    })?;

    // Resume / --continue: the JSONL already contains prior turns, so the
    // tailer must seek to EOF (otherwise we'd replay old records). We learn
    // this from the options — checking `before.contains(stem)` would be
    // wrong because Claude's project-dir encoding doesn't always match ours
    // (it also collapses `_` in path components).
    let resume_mode = options.resume.is_some() || options.continue_conversation == Some(true);

    let before = list_jsonl_stems(&project_dir).await?;

    let harness = Arc::new(HookHarness::create()?);
    inject_fifo_env(&mut options, &harness);

    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();

    // Open the FIFO reader BEFORE spawning claude — otherwise the child's
    // first hook write blocks waiting for a reader to attach.
    let (hook_tx, hook_rx) = mpsc::unbounded_channel::<HookLine>();
    let _reader_handle = spawn_reader(
        harness.fifo_path.clone(),
        hook_tx,
        cancellation_token.clone(),
    )?;

    // Spawn the hook listener BEFORE the session so any hook lines that
    // arrive immediately after spawn are captured.
    let (_listener_handle, state_rx) = spawn_hook_listener(hook_rx, cancellation_token.clone());

    let session =
        Arc::new(TmuxSession::spawn(&options, Some(harness.settings_json.as_str())).await?);

    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
        socket: session.socket.clone(),
    });

    // Trust-dialog watcher: tmux pane → strip CSI → match "trust"+"folder".
    // Auto-stops once `SessionStart` fires (cancellation token below). It also
    // publishes each pane capture on `pane_rx` so startup failures can log the
    // last thing claude printed before dying.
    let trust_token = CancellationToken::new();
    let trust_writer = Arc::clone(&writer);
    let trust_session = Arc::clone(&session);
    let (pane_tx, pane_rx) = watch::channel(Vec::new());
    let trust_handle = tokio::spawn(startup_dialog_watcher(
        trust_session,
        trust_writer,
        trust_token.clone(),
        pane_tx,
    ));

    // Wait for SessionStart (TUI is up + readline accepting input). Honors
    // `options.init_timeout` (default 60s). The trust watcher runs in parallel
    // and is cancelled the moment we proceed.
    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    let wait_result = wait_for_session_start(state_rx.clone(), &session, init_timeout).await;
    trust_token.cancel();
    let _ = trust_handle.await;

    if let Err(e) = wait_result {
        let pane = capture_startup_pane(&session, &pane_rx).await;
        log_startup_failure(&session, &pane, &e);
        session.kill().await;
        return Err(e.with_pane_tail(pane));
    }

    if let Err(e) = writer.send_text(prompt).await {
        session.kill().await;
        return Err(e);
    }

    // Resume (`--resume <id>`) and `--continue` both append to an EXISTING
    // transcript, so snapshot-diff never sees a new file — target it directly
    // instead of waiting on the turn-end Stop hook (which times out on long
    // first turns). See [`resolve_resume_target`].
    let resume_target = resolve_resume_target(&options, &project_dir).await;

    let jsonl_path = match discover_transcript_path(
        state_rx.clone(),
        project_dir.clone(),
        before,
        resume_target,
        init_timeout,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            session.kill().await;
            return Err(e);
        }
    };
    let start_at_end = resume_mode;
    if resume_mode {
        debug!(path = %jsonl_path.display(), "Resume mode — tailing from EOF");
    }

    let Some(session_id) = jsonl_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(std::string::ToString::to_string)
    else {
        session.kill().await;
        return Err(crate::error::SdkError::Internal(format!(
            "JSONL filename has no UTF-8 stem: {}",
            jsonl_path.display()
        )));
    };

    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    // Synthetic SystemInit so consumers (sandbox, etc.) get the session id +
    // model up front, before any JSONL records flow.
    let init = synth_system_init(&session_id, &options);
    if tx.send(Ok(SdkMessage::System(init))).await.is_err() {
        debug!("Consumer dropped channel before init delivery");
    }

    let target = TailTarget {
        path: jsonl_path,
        session_id,
        start_at_end,
    };
    let join_handle = spawn_supervisor(
        Arc::clone(&session),
        target,
        tx,
        cancellation_token.clone(),
        state_rx,
    );

    anchor_harness(harness, cancellation_token.clone());

    let stdin_writer: Arc<dyn StdinWriter> = writer;
    Ok(QueryHandle::from_parts(
        rx,
        join_handle,
        stdin_writer,
        cancellation_token,
    ))
}

/// Multi-turn tmux client. Spawns the TUI and returns immediately — caller
/// drives the conversation via [`QueryHandle::send_user_message`]. The
/// supervisor task discovers the JSONL transcript lazily after the caller's
/// first message creates it.
pub async fn query_tmux_streaming(mut options: ClaudeOptions) -> Result<QueryHandle> {
    warn_on_ignored_options(&options);

    let project_dir = resolve_project_dir(&options)?;
    tokio::fs::create_dir_all(&project_dir).await.map_err(|e| {
        crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        ))
    })?;

    let before = list_jsonl_stems(&project_dir).await?;

    let harness = Arc::new(HookHarness::create()?);
    inject_fifo_env(&mut options, &harness);

    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();

    // Open the FIFO reader BEFORE spawning claude — otherwise the child's
    // first hook write blocks waiting for a reader to attach.
    let (hook_tx, hook_rx) = mpsc::unbounded_channel::<HookLine>();
    let _reader_handle = spawn_reader(
        harness.fifo_path.clone(),
        hook_tx,
        cancellation_token.clone(),
    )?;

    let (_listener_handle, state_rx) = spawn_hook_listener(hook_rx, cancellation_token.clone());

    let session =
        Arc::new(TmuxSession::spawn(&options, Some(harness.settings_json.as_str())).await?);

    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
        socket: session.socket.clone(),
    });

    // Trust watcher + SessionStart wait. Mirrors `query_tmux`.
    let trust_token = CancellationToken::new();
    let trust_writer = Arc::clone(&writer);
    let trust_session = Arc::clone(&session);
    let (pane_tx, pane_rx) = watch::channel(Vec::new());
    let trust_handle = tokio::spawn(startup_dialog_watcher(
        trust_session,
        trust_writer,
        trust_token.clone(),
        pane_tx,
    ));
    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    let wait_result = wait_for_session_start(state_rx.clone(), &session, init_timeout).await;
    trust_token.cancel();
    let _ = trust_handle.await;
    if let Err(e) = wait_result {
        let pane = capture_startup_pane(&session, &pane_rx).await;
        log_startup_failure(&session, &pane, &e);
        session.kill().await;
        return Err(e.with_pane_tail(pane));
    }

    // Captured once the session is up: the CLI process in the pane. Callers
    // use it to separate the persistent session from background work it
    // spawned. Valid for this handle's lifetime — a respawned TUI means a new
    // handle.
    let pane_pid = session.pane_pid().await;

    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    let join_handle = spawn_lazy_supervisor(
        Arc::clone(&session),
        project_dir,
        before,
        options,
        tx,
        cancellation_token.clone(),
        state_rx,
    );

    anchor_harness(harness, cancellation_token.clone());

    let stdin_writer: Arc<dyn StdinWriter> = writer;
    Ok(
        QueryHandle::from_parts(rx, join_handle, stdin_writer, cancellation_token)
            .with_tmux_pane_pid(pane_pid),
    )
}

fn warn_on_ignored_options(options: &ClaudeOptions) {
    if options.can_use_tool.is_some() {
        warn!(
            "tmux mode: ClaudeOptions.can_use_tool is set but will be ignored \
             (permission interception is not supported in TUI mode)"
        );
    }
}

/// Inject `CLAUDE_P_FIFO=<path>` into `options.env` so the relay script (run
/// by claude when a hook fires) can locate the FIFO. The env map is created
/// on demand.
fn inject_fifo_env(options: &mut ClaudeOptions, harness: &HookHarness) {
    let env = options
        .env
        .get_or_insert_with(std::collections::HashMap::new);
    env.insert(
        "CLAUDE_P_FIFO".to_string(),
        harness.fifo_path.display().to_string(),
    );
}

/// Detach a no-op task that keeps `harness` alive until the cancellation
/// token fires. This guarantees the FIFO and relay script remain on disk
/// while the supervisor + hook listener are still using them — `Drop` only
/// runs when the last `Arc` reference is dropped.
fn anchor_harness(harness: Arc<HookHarness>, cancellation_token: CancellationToken) {
    tokio::spawn(async move {
        cancellation_token.cancelled().await;
        drop(harness);
    });
}

/// Upper bound on pane output included in startup-failure logs.
const PANE_CAPTURE_MAX_BYTES: usize = 2048;

/// Capture the tmux pane at startup-failure time, for both the log line and
/// embedding in the returned error (see [`SdkError::with_pane_tail`]).
///
/// Tries a live capture first — that works for [`SdkError::SessionStartTimeout`],
/// where the session is still alive but stuck. For [`SdkError::TmuxSessionDied`]
/// the session is already gone, so this falls back to the last snapshot taken
/// by [`startup_dialog_watcher`]'s 250ms capture loop. Returns the lossy tail
/// (capped at [`PANE_CAPTURE_MAX_BYTES`]), or `None` when only whitespace (or
/// nothing) was captured.
async fn capture_startup_pane(
    session: &TmuxSession,
    last_pane: &watch::Receiver<Vec<u8>>,
) -> Option<String> {
    let live = session
        .capture_pane_stripped()
        .await
        .filter(|b| !b.iter().all(u8::is_ascii_whitespace));
    let bytes = match live {
        Some(bytes) => bytes,
        None => last_pane.borrow().clone(),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    Some(pane_tail_lossy(&bytes, PANE_CAPTURE_MAX_BYTES))
}

/// Log the captured tmux pane contents when the session fails to reach
/// `SessionStart`, so the reason claude exited (e.g. "No conversation found
/// with session ID …") lands in the logs instead of being lost with the pane.
/// The same `pane` is attached to the returned error so it also survives in the
/// task's error event.
fn log_startup_failure(session: &TmuxSession, pane: &Option<String>, error: &SdkError) {
    if let Some(tail) = pane {
        warn!(
            session_name = %session.session_name,
            error = %error,
            pane_output = %tail,
            "tmux session failed to start — captured pane output"
        );
    } else {
        warn!(
            session_name = %session.session_name,
            error = %error,
            "tmux session failed to start; no pane output captured"
        );
    }
}

/// Render the tail of captured pane bytes as lossy UTF-8, capped at
/// `max_bytes` (the most recent output is the most useful part).
fn pane_tail_lossy(bytes: &[u8], max_bytes: usize) -> String {
    let start = bytes.len().saturating_sub(max_bytes);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let trimmed = text.trim();
    if start > 0 {
        format!("…{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Interval at which [`wait_for_session_start`] polls the tmux session's
/// liveness while waiting for the `SessionStart` hook.
const LIVENESS_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Wait until [`HookState::SessionStarted`] is observed on the watch channel,
/// or until `timeout` elapses.
///
/// While waiting, the tmux session's liveness is polled every
/// [`LIVENESS_POLL_INTERVAL`]. If the session dies before `SessionStart` fires
/// — e.g. `claude --continue` prints "No conversation found to continue" and
/// exits immediately — this returns [`SdkError::TmuxSessionDied`] within one
/// poll interval instead of blocking for the full `timeout`. Callers (and the
/// sandbox's fresh-session fallback) can then retry quickly.
async fn wait_for_session_start(
    mut state_rx: watch::Receiver<HookState>,
    session: &TmuxSession,
    timeout: Duration,
) -> Result<()> {
    // The pane is captured by the caller's failure handler (which has the
    // watcher's last snapshot); these constructors leave it empty.
    let timeout_err = || SdkError::SessionStartTimeout {
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        pane_tail: None,
    };
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(*state_rx.borrow(), HookState::SessionStarted { .. }) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timeout_err());
        }
        // Wake on a hook-state change, but no later than the next liveness poll
        // (and never past the overall deadline).
        let wait = remaining.min(LIVENESS_POLL_INTERVAL);
        match tokio::time::timeout(wait, state_rx.changed()).await {
            // State changed — re-check at the top of the loop.
            Ok(Ok(())) => continue,
            // Sender dropped — listener exited before SessionStart fired.
            Ok(Err(_)) => return Err(timeout_err()),
            // Poll tick (or final slice) elapsed — check the session is still up.
            Err(_) => {
                if !session.is_alive().await {
                    return Err(SdkError::TmuxSessionDied { pane_tail: None });
                }
            }
        }
    }
}

/// Poll `tmux capture-pane -p` every [`TRUST_POLL_INTERVAL`] looking for the
/// workspace-trust dialog. When detected, send Enter (the default selection
/// is "Yes, I trust this folder") and exit.
///
/// Cancels itself when `cancellation_token` fires — the caller cancels right
/// after `SessionStart` fires, so this task is only active during the brief
/// pre-SessionStart window.
async fn startup_dialog_watcher(
    session: Arc<TmuxSession>,
    writer: Arc<TmuxPasteWriter>,
    cancellation_token: CancellationToken,
    pane_tx: watch::Sender<Vec<u8>>,
) {
    // Two distinct blocking dialogs can appear before SessionStart, in order:
    //   1. workspace-trust ("…trust the files in this folder?") — default is
    //      "Yes", a bare Enter accepts.
    //   2. bypass-permissions warning (from --dangerously-skip-permissions) —
    //      default is "No, exit", so we must move Down to "Yes, I accept" first.
    // Each is dismissed at most once; we keep polling until both are handled or
    // SessionStart cancels us.
    let mut trust_dismissed = false;
    let mut bypass_dismissed = false;
    loop {
        if trust_dismissed && bypass_dismissed {
            return;
        }
        tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("startup_dialog_watcher: cancelled");
                return;
            }
            () = tokio::time::sleep(TRUST_POLL_INTERVAL) => {}
        }

        // capture_pane_stripped returns None when the session is dying —
        // don't spam logs, just retry on the next tick.
        let Some(stripped) = session.capture_pane_stripped().await else {
            continue;
        };
        // Keep the last non-empty capture around: when the session dies before
        // SessionStart, the pane is unrecoverable afterwards, so this snapshot
        // is the only record of what claude printed before exiting.
        if !stripped.iter().all(u8::is_ascii_whitespace) {
            let _ = pane_tx.send(stripped.clone());
        }

        if !trust_dismissed && scan_for_trust_dialog(&stripped) {
            debug!("startup_dialog_watcher: trust dialog detected — pressing Enter");
            if let Err(e) = writer.send_enter().await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Enter for trust");
            }
            trust_dismissed = true;
            continue;
        }

        if !bypass_dismissed && scan_for_bypass_dialog(&stripped) {
            debug!("startup_dialog_watcher: bypass-permissions dialog detected — selecting accept");
            // Default selection is "No, exit"; move Down to "Yes, I accept".
            if let Err(e) = writer.send_key("Down").await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Down for bypass");
            } else if let Err(e) = writer.send_enter().await {
                warn!(error = %e, "startup_dialog_watcher: failed to send Enter for bypass");
            }
            bypass_dismissed = true;
            continue;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_tail_lossy_short_input_passes_through_trimmed() {
        assert_eq!(pane_tail_lossy(b"  hello world \n", 2048), "hello world");
    }

    #[test]
    fn pane_tail_lossy_truncates_to_tail_with_ellipsis() {
        let bytes = [b"x".repeat(100), b"recent output".to_vec()].concat();
        let out = pane_tail_lossy(&bytes, 13);
        assert_eq!(out, "…recent output");
    }

    #[test]
    fn pane_tail_lossy_handles_invalid_utf8() {
        let out = pane_tail_lossy(&[0xff, 0xfe, b'o', b'k'], 2048);
        assert!(
            out.ends_with("ok"),
            "lossy conversion should keep ascii tail: {out}"
        );
    }
}

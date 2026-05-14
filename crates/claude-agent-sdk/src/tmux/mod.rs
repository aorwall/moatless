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
//! Workspace-trust dialog: an unfamiliar cwd causes the TUI to display
//! "Do you trust the files in this folder?" before SessionStart fires.
//! [`trust_dialog_watcher`] polls `tmux capture-pane` every 250ms during the
//! pre-SessionStart window and sends Enter when both "trust" and "folder"
//! appear in the stripped output — the default selection accepts.

mod args;
mod hooks;
mod session;
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

use self::hooks::{spawn_reader, HookHarness, HookLine};
use self::session::{list_jsonl_stems, resolve_project_dir, TmuxSession};
use self::tailer::{
    discover_transcript_path, spawn_hook_listener, spawn_lazy_supervisor, spawn_supervisor,
    synth_system_init, HookState, TailTarget,
};
use self::trust::{scan_for_trust_dialog, strip_csi};
use self::writer::TmuxPasteWriter;

/// Channel capacity for the message stream. Matches the subprocess path.
const CHANNEL_CAPACITY: usize = 256;

/// Default upper bound on waiting for the `SessionStart` hook to fire.
/// Overridden by `ClaudeOptions.init_timeout`.
const DEFAULT_SESSION_START_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the trust-dialog watcher captures the tmux pane while waiting
/// for SessionStart. Only active in the brief pre-SessionStart window; auto-
/// stops once either SessionStart fires or the dialog is dismissed.
const TRUST_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// One-shot tmux query. Spawns the TUI, pastes `prompt` as the first user
/// message, and returns a `QueryHandle` for consuming the response stream.
pub async fn query_tmux(prompt: &str, mut options: ClaudeOptions) -> Result<QueryHandle> {
    warn_on_ignored_options(&options);

    let project_dir = resolve_project_dir(&options)?;
    tokio::fs::create_dir_all(&project_dir)
        .await
        .map_err(|e| crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        )))?;

    // Resume / --continue: the JSONL already contains prior turns, so the
    // tailer must seek to EOF (otherwise we'd replay old records). We learn
    // this from the options — checking `before.contains(stem)` would be
    // wrong because Claude's project-dir encoding doesn't always match ours
    // (it also collapses `_` in path components).
    let resume_mode =
        options.resume.is_some() || options.continue_conversation == Some(true);

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
    let (_listener_handle, state_rx) =
        spawn_hook_listener(hook_rx, cancellation_token.clone());

    let session = Arc::new(
        TmuxSession::spawn(&options, Some(harness.settings_json.as_str())).await?,
    );

    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
    });

    // Trust-dialog watcher: tmux pane → strip CSI → match "trust"+"folder".
    // Auto-stops once `SessionStart` fires (cancellation token below).
    let trust_token = CancellationToken::new();
    let trust_writer = Arc::clone(&writer);
    let trust_session = Arc::clone(&session);
    let trust_handle = tokio::spawn(trust_dialog_watcher(
        trust_session,
        trust_writer,
        trust_token.clone(),
    ));

    // Wait for SessionStart (TUI is up + readline accepting input). Honors
    // `options.init_timeout` (default 60s). The trust watcher runs in parallel
    // and is cancelled the moment we proceed.
    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    let wait_result = wait_for_session_start(state_rx.clone(), init_timeout).await;
    trust_token.cancel();
    let _ = trust_handle.await;

    if let Err(e) = wait_result {
        session.kill().await;
        return Err(e);
    }

    if let Err(e) = writer.send_text(prompt).await {
        session.kill().await;
        return Err(e);
    }

    let jsonl_path =
        match discover_transcript_path(state_rx.clone(), project_dir.clone(), before, init_timeout)
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
    tokio::fs::create_dir_all(&project_dir)
        .await
        .map_err(|e| crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        )))?;

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

    let (_listener_handle, state_rx) =
        spawn_hook_listener(hook_rx, cancellation_token.clone());

    let session = Arc::new(
        TmuxSession::spawn(&options, Some(harness.settings_json.as_str())).await?,
    );

    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
    });

    // Trust watcher + SessionStart wait. Mirrors `query_tmux`.
    let trust_token = CancellationToken::new();
    let trust_writer = Arc::clone(&writer);
    let trust_session = Arc::clone(&session);
    let trust_handle = tokio::spawn(trust_dialog_watcher(
        trust_session,
        trust_writer,
        trust_token.clone(),
    ));
    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    let wait_result = wait_for_session_start(state_rx.clone(), init_timeout).await;
    trust_token.cancel();
    let _ = trust_handle.await;
    if let Err(e) = wait_result {
        session.kill().await;
        return Err(e);
    }

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
    Ok(QueryHandle::from_parts(
        rx,
        join_handle,
        stdin_writer,
        cancellation_token,
    ))
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
    let env = options.env.get_or_insert_with(std::collections::HashMap::new);
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

/// Wait until [`HookState::SessionStarted`] is observed on the watch channel,
/// or until `timeout` elapses.
async fn wait_for_session_start(
    mut state_rx: watch::Receiver<HookState>,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(*state_rx.borrow(), HookState::SessionStarted { .. }) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(SdkError::SessionStartTimeout {
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            });
        }
        match tokio::time::timeout(remaining, state_rx.changed()).await {
            Ok(Ok(())) => continue,
            Ok(Err(_)) => {
                // Sender dropped — listener exited before SessionStart fired.
                return Err(SdkError::SessionStartTimeout {
                    timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                });
            }
            Err(_) => {
                return Err(SdkError::SessionStartTimeout {
                    timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                });
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
async fn trust_dialog_watcher(
    session: Arc<TmuxSession>,
    writer: Arc<TmuxPasteWriter>,
    cancellation_token: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("trust_dialog_watcher: cancelled");
                return;
            }
            () = tokio::time::sleep(TRUST_POLL_INTERVAL) => {}
        }

        let output = tokio::process::Command::new("tmux")
            .args(["capture-pane", "-t", session.session_name.as_str(), "-p"])
            .stderr(std::process::Stdio::null())
            .output()
            .await;
        let Ok(out) = output else {
            // tmux command failed (session might be dying). Don't spam logs.
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let stripped = strip_csi(&out.stdout);
        if scan_for_trust_dialog(&stripped) {
            debug!("trust_dialog_watcher: dialog detected — pressing Enter");
            if let Err(e) = writer.send_enter().await {
                warn!(error = %e, "trust_dialog_watcher: failed to send Enter");
            }
            return;
        }
    }
}

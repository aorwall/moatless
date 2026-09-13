use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::api::messages::SdkMessage;
use crate::api::options::ClaudeOptions;
use crate::error::Result;
use crate::query::QueryHandle;
use crate::transport::StdinWriter;

use super::hooks::{HookHarness, HookLine, spawn_reader};
use super::session::TmuxSession;
use super::startup::await_readiness;
use super::tailer::{
    TailTarget, discover_transcript_path, spawn_hook_listener, spawn_lazy_supervisor,
    spawn_supervisor, synth_system_init,
};
use super::writer::TmuxPasteWriter;
use crate::transcripts::{list_jsonl_stems, resolve_project_dir, resolve_resume_target};

/// Channel capacity for the message stream. Matches the subprocess path.
const CHANNEL_CAPACITY: usize = 256;

/// Default upper bound on waiting for the `SessionStart` hook to fire.
/// Overridden by `ClaudeOptions.init_timeout`.
const DEFAULT_SESSION_START_TIMEOUT: Duration = Duration::from_mins(1);

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

    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    await_readiness(&session, &writer, state_rx.clone(), init_timeout).await?;

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

/// Starts a multi-turn tmux client, spawning the TUI and returning immediately.
///
/// The caller drives the conversation via [`QueryHandle::send_user_message`];
/// the supervisor discovers the JSONL transcript lazily, after the first
/// message creates it.
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

    let init_timeout = options
        .init_timeout
        .unwrap_or(DEFAULT_SESSION_START_TIMEOUT);
    await_readiness(&session, &writer, state_rx.clone(), init_timeout).await?;

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

/// Inject `CLAUDE_P_FIFO=<path>` into `options.env` so the relay script (run by
/// claude when a hook fires) can locate the FIFO. The env map is created on
/// demand.
fn inject_fifo_env(options: &mut ClaudeOptions, harness: &HookHarness) {
    let env = options
        .env
        .get_or_insert_with(std::collections::HashMap::new);
    env.insert(
        "CLAUDE_P_FIFO".to_string(),
        harness.fifo_path.display().to_string(),
    );
}

/// Detach a no-op task that keeps `harness` alive until the cancellation token
/// fires.
///
/// Guarantees the FIFO and relay script stay on disk while the supervisor and
/// hook listener use them — `Drop` runs only on the last `Arc`.
fn anchor_harness(harness: Arc<HookHarness>, cancellation_token: CancellationToken) {
    tokio::spawn(async move {
        cancellation_token.cancelled().await;
        drop(harness);
    });
}

//! Tmux-backed query mode.
//!
//! Runs the real interactive Claude TUI inside a detached tmux session and
//! exposes the same `QueryHandle` shape as the subprocess backend. Use this
//! when you need the full TUI experience (skills, slash commands, plugins
//! that don't fire in `--print` mode).
//!
//! Trade-offs vs the subprocess backend:
//! - Permissions are always bypassed (`--dangerously-skip-permissions`).
//!   `ClaudeOptions.can_use_tool` is ignored.
//! - SDK MCP servers (`sdk_mcp_servers`) are not supported — the TUI owns its
//!   own MCP lifecycle.
//! - No streaming partials. Messages arrive once the TUI flushes complete
//!   blocks to the JSONL transcript.
//! - **The TUI does not create a JSONL transcript until the first user
//!   message is submitted.** [`query_tmux`] handles this by pasting the
//!   prompt before waiting for the file; [`query_tmux_streaming`] starts a
//!   lazy supervisor that polls until the caller's first
//!   [`QueryHandle::send_user_message`] triggers file creation.

mod args;
mod session;
mod tailer;
mod translator;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::error::Result;
use crate::query::QueryHandle;
use crate::transport::StdinWriter;
use crate::types::messages::SdkMessage;
use crate::types::options::ClaudeOptions;

use self::session::{discover_new_jsonl, list_jsonl_stems, resolve_project_dir, TmuxSession};
use self::tailer::{spawn_lazy_supervisor, spawn_supervisor, synth_system_init, TailTarget};
use self::writer::TmuxPasteWriter;

/// Channel capacity for the message stream. Matches the subprocess path.
const CHANNEL_CAPACITY: usize = 256;

/// Time to wait after `tmux new-session` returns before pasting the first
/// prompt, so the TUI readline loop has a chance to come up.
const TUI_READY_DELAY: Duration = Duration::from_millis(500);

/// One-shot tmux query. Spawns the TUI, pastes `prompt` as the first user
/// message, and returns a `QueryHandle` for consuming the response stream.
pub async fn query_tmux(prompt: &str, options: ClaudeOptions) -> Result<QueryHandle> {
    warn_on_ignored_options(&options);

    let project_dir = resolve_project_dir(&options)?;
    tokio::fs::create_dir_all(&project_dir)
        .await
        .map_err(|e| crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        )))?;

    // Resume case: if the JSONL already exists, we'll seek to its end and tail
    // it directly (no snapshot diff needed).
    let resume_target = options.resume.as_deref().and_then(|id| {
        let candidate = project_dir.join(format!("{id}.jsonl"));
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    });

    let before = list_jsonl_stems(&project_dir).await?;

    let session = Arc::new(TmuxSession::spawn(&options).await?);
    tokio::time::sleep(TUI_READY_DELAY).await;

    // Paste the initial prompt. After this the TUI starts (or appends to) the
    // JSONL transcript.
    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
    });
    if let Err(e) = writer.send_text(prompt).await {
        session.kill().await;
        return Err(e);
    }

    let init_timeout = options
        .init_timeout
        .unwrap_or(Duration::from_secs(60));

    let (jsonl_path, start_at_end) = if let Some(path) = resume_target {
        debug!(path = %path.display(), "Resuming existing JSONL");
        (path, true)
    } else {
        match discover_new_jsonl(&project_dir, &before, init_timeout).await {
            Ok(p) => (p, false),
            Err(e) => {
                session.kill().await;
                return Err(e);
            }
        }
    };

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

    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
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
    let supervisor = spawn_supervisor(
        Arc::clone(&session),
        target,
        tx,
        cancellation_token.clone(),
    );

    let stdin_writer: Arc<dyn StdinWriter> = writer;
    Ok(QueryHandle::from_parts(
        rx,
        supervisor,
        stdin_writer,
        cancellation_token,
    ))
}

/// Multi-turn tmux client. Spawns the TUI and returns immediately — caller
/// drives the conversation via [`QueryHandle::send_user_message`]. The
/// supervisor task discovers the JSONL transcript lazily after the caller's
/// first message creates it.
pub async fn query_tmux_streaming(options: ClaudeOptions) -> Result<QueryHandle> {
    warn_on_ignored_options(&options);

    let project_dir = resolve_project_dir(&options)?;
    tokio::fs::create_dir_all(&project_dir)
        .await
        .map_err(|e| crate::error::SdkError::Internal(format!(
            "Failed to create project dir {}: {e}",
            project_dir.display()
        )))?;

    let before = list_jsonl_stems(&project_dir).await?;

    let session = Arc::new(TmuxSession::spawn(&options).await?);
    tokio::time::sleep(TUI_READY_DELAY).await;

    let writer = Arc::new(TmuxPasteWriter {
        session_name: session.session_name.clone(),
    });

    let cancellation_token = options.cancellation_token.clone().unwrap_or_default();
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    let supervisor = spawn_lazy_supervisor(
        Arc::clone(&session),
        project_dir,
        before,
        options,
        tx,
        cancellation_token.clone(),
    );

    let stdin_writer: Arc<dyn StdinWriter> = writer;
    Ok(QueryHandle::from_parts(
        rx,
        supervisor,
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

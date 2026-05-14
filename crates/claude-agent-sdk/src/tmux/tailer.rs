//! Background task that poll-tails the JSONL transcript file and forwards
//! translated `SdkMessage`s to the consumer channel.
//!
//! Companion task: a liveness monitor that runs `tmux has-session` every 5s
//! and signals if the tmux session disappears.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::error::{Result, SdkError};
use crate::types::messages::{ApiKeySource, SdkMessage, SystemInitMessage};
use crate::types::options::ClaudeOptions;

use super::session::{discover_new_jsonl, TmuxSession};
use super::translator::{translate_record, Translated, TurnAccumulator};

/// Inputs needed to start tailing once we know the JSONL path + session id.
pub(crate) struct TailTarget {
    pub(crate) path: PathBuf,
    pub(crate) session_id: String,
    pub(crate) start_at_end: bool,
}

/// Spawn the supervisor task that owns the tmux session, runs the tailer and
/// liveness monitor, and kills the session on cancellation.
///
/// `target` is the already-discovered JSONL file (one-shot path: paste prompt,
/// wait for file to appear, then spawn this).
pub(crate) fn spawn_supervisor(
    session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_supervisor(session, target, tx, cancellation_token).await;
    })
}

/// Lazy variant — used by `query_tmux_streaming` where we don't yet know the
/// session id (no prompt has been pasted yet). The supervisor polls the
/// project dir until a new JSONL appears, then transitions to tailing.
pub(crate) fn spawn_lazy_supervisor(
    session: Arc<TmuxSession>,
    project_dir: PathBuf,
    before: HashSet<String>,
    options: ClaudeOptions,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Honor options.init_timeout as an upper bound, but if the user never
        // pastes anything we just sit and wait until cancellation.
        let timeout = options
            .init_timeout
            .unwrap_or(Duration::from_secs(3600));

        let discover = tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("lazy supervisor cancelled before JSONL discovery");
                session.kill().await;
                return;
            }
            res = discover_new_jsonl(&project_dir, &before, timeout) => res,
        };

        let jsonl_path = match discover {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "lazy supervisor: JSONL discovery failed");
                let _ = tx.send(Err(e)).await;
                session.kill().await;
                return;
            }
        };

        let Some(session_id) = jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(std::string::ToString::to_string)
        else {
            let _ = tx
                .send(Err(SdkError::Internal(format!(
                    "JSONL filename has no UTF-8 stem: {}",
                    jsonl_path.display()
                ))))
                .await;
            session.kill().await;
            return;
        };

        let init = synth_system_init(&session_id, &options);
        if tx.send(Ok(SdkMessage::System(init))).await.is_err() {
            debug!("Consumer dropped before init delivery");
            session.kill().await;
            return;
        }

        let target = TailTarget {
            path: jsonl_path,
            session_id,
            start_at_end: false,
        };
        run_supervisor(session, target, tx, cancellation_token).await;
    })
}

async fn run_supervisor(
    session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) {
    let tailer_tx = tx.clone();
    let tailer_token = cancellation_token.clone();
    let tailer_session = Arc::clone(&session);
    let tailer_target = TailTarget {
        path: target.path.clone(),
        session_id: target.session_id.clone(),
        start_at_end: target.start_at_end,
    };
    let tailer_task = tokio::spawn(async move {
        tail_loop(tailer_session, tailer_target, tailer_tx, tailer_token).await;
    });

    let liveness_tx = tx.clone();
    let liveness_token = cancellation_token.clone();
    let liveness_session = Arc::clone(&session);
    let liveness_task = tokio::spawn(async move {
        liveness_loop(liveness_session, liveness_tx, liveness_token).await;
    });

    // Wait for cancellation OR for both child tasks to exit on their own.
    tokio::select! {
        () = cancellation_token.cancelled() => {
            debug!("Supervisor: cancellation received");
        }
        () = async {
            let _ = tailer_task.await;
            let _ = liveness_task.await;
        } => {
            debug!("Supervisor: both child tasks exited");
            cancellation_token.cancel();
        }
    }

    cancellation_token.cancel();
    session.kill().await;
    info!(
        session_name = %session.session_name,
        "tmux supervisor exited and session killed"
    );
}

async fn tail_loop(
    _session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) {
    let path = target.path;
    let session_id = target.session_id;
    let start_at_end = target.start_at_end;

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx
                .send(Err(SdkError::Internal(format!(
                    "Failed to open JSONL {}: {e}",
                    path.display()
                ))))
                .await;
            return;
        }
    };
    let mut reader = BufReader::new(file);
    if start_at_end {
        if let Err(e) = reader.seek(SeekFrom::End(0)).await {
            warn!(error = %e, "Failed to seek to end of JSONL — will tail from start");
        } else {
            debug!(path = %path.display(), "Resumed JSONL — tailing from EOF");
        }
    }

    let mut acc = TurnAccumulator::new(session_id);
    let mut buf = String::new();
    let eof_sleep = Duration::from_millis(100);

    loop {
        if cancellation_token.is_cancelled() {
            return;
        }

        buf.clear();
        let read_fut = reader.read_line(&mut buf);
        let n = tokio::select! {
            () = cancellation_token.cancelled() => return,
            res = read_fut => match res {
                Ok(n) => n,
                Err(e) => {
                    warn!(error = %e, "JSONL read error");
                    let _ = tx.send(Err(SdkError::Internal(format!("JSONL read error: {e}")))).await;
                    return;
                }
            }
        };

        if n == 0 {
            // EOF — wait for more data
            tokio::select! {
                () = cancellation_token.cancelled() => return,
                () = tokio::time::sleep(eof_sleep) => continue,
            }
        }

        let line = buf.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, line_len = line.len(), "Skipping malformed JSONL line");
                continue;
            }
        };

        match translate_record(value, &mut acc) {
            Translated::Skip => {}
            Translated::One(msg) => {
                if tx.send(Ok(*msg)).await.is_err() {
                    debug!("Consumer dropped — tailer exiting");
                    return;
                }
            }
        }
    }
}

async fn liveness_loop(
    session: Arc<TmuxSession>,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) {
    let interval = Duration::from_secs(5);
    loop {
        tokio::select! {
            () = cancellation_token.cancelled() => return,
            () = tokio::time::sleep(interval) => {}
        }
        if cancellation_token.is_cancelled() {
            return;
        }
        if !session.is_alive().await {
            warn!(
                session_name = %session.session_name,
                "tmux session disappeared"
            );
            let _ = tx.send(Err(SdkError::TmuxSessionDied)).await;
            cancellation_token.cancel();
            return;
        }
    }
}

/// Build a synthetic `SystemInit` message so consumers get the session id and
/// model up front. The JSONL transcript has no equivalent record.
pub(crate) fn synth_system_init(session_id: &str, options: &ClaudeOptions) -> SystemInitMessage {
    SystemInitMessage {
        uuid: format!("tmux-init-{session_id}"),
        session_id: session_id.to_string(),
        tools: Vec::new(),
        model: options.model.clone().unwrap_or_default(),
        mcp_servers: Vec::new(),
        permission_mode: crate::types::permissions::PermissionMode::BypassPermissions,
        cwd: options.cwd.clone().unwrap_or_default(),
        api_key_source: ApiKeySource::None,
        slash_commands: Vec::new(),
        output_style: crate::types::messages::OutputStyle::Normal,
        skills: Vec::new(),
        plugins: Vec::new(),
        agents: None,
        betas: options.betas.clone(),
        claude_code_version: String::new(),
    }
}

#[allow(dead_code)]
fn _path_unused(_p: &Path) {}

//! Background task that poll-tails the JSONL transcript file and forwards
//! translated `SdkMessage`s to the consumer channel.
//!
//! Companion task: a liveness monitor that runs `tmux has-session` every 5s and
//! signals if the tmux session disappears.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::api::messages::{ApiKeySource, SdkMessage, SystemInitMessage};
use crate::api::options::ClaudeOptions;
use crate::error::{Result, SdkError};

use super::hooks::{
    HookEvent, HookLine, extract_last_assistant_message, extract_session_id,
    extract_transcript_path,
};
use super::session::TmuxSession;
use super::subagents::{
    SubagentRegistry, spawn_subagent_watcher, subagent_dir, sum_transcript_usage,
};
use super::translator::{
    Translated, TurnAccumulator, extract_subagent_completion, translate_record,
    translate_stop_payload_fallback,
};
use crate::transcripts::list_jsonl_stems;

/// Retry budget for the Stop fallback: wait up to 40 × 50ms = 2s for the
/// `tail_loop` to emit a `Result` from the JSONL `turn_duration` record before
/// synthesizing one from the Stop payload's `last_assistant_message`.
const STOP_FALLBACK_RETRIES: u32 = 40;
const STOP_FALLBACK_POLL: Duration = Duration::from_millis(50);

/// State the hook listener publishes via [`tokio::sync::watch`]. Consumers
/// (mod.rs wait-for-SessionStart; transcript discovery race) clone the receiver
/// and await updates.
//
// reason: the `session_id` payload on `SessionStarted` and the
// `last_assistant_message` field on `TurnEnded` are consumed in step 7
// (Result fallback). Until then they're staged.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) enum HookState {
    /// No hooks fired yet.
    #[default]
    Pending,
    /// `SessionStart` hook fired — the TUI is up and accepting input.
    SessionStarted { session_id: Option<String> },
    /// `Stop` hook fired — turn finished. May fire multiple times for a
    /// streaming session; each update overwrites the previous one.
    TurnEnded {
        transcript_path: Option<PathBuf>,
        last_assistant_message: Option<String>,
        session_id: Option<String>,
    },
}

/// Inputs needed to start tailing once we know the JSONL path + session id.
pub(crate) struct TailTarget {
    /// The already-discovered JSONL transcript file to tail.
    pub(crate) path: PathBuf,
    pub(crate) session_id: String,
    pub(crate) start_at_end: bool,
}

/// Race the hook-driven `Stop.transcript_path` against snapshot-diff against
/// `before`, returning whichever resolves first (else
/// [`SdkError::JsonlNotFound`] on timeout).
///
/// Stop is authoritative and covers `--continue`/`--resume`, where claude
/// appends to an existing file and snapshot-diff sees nothing new;
/// snapshot-diff is a defensive fallback for older Claude versions and fresh
/// sessions.
pub(crate) async fn discover_transcript_path(
    mut state_rx: watch::Receiver<HookState>,
    project_dir: PathBuf,
    before: HashSet<String>,
    resume_target: Option<PathBuf>,
    timeout: Duration,
) -> Result<PathBuf> {
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(200);

    loop {
        // Check the current watch state — Stop may have fired before we
        // started awaiting changes.
        if let HookState::TurnEnded {
            transcript_path: Some(p),
            ..
        } = &*state_rx.borrow()
        {
            return Ok(p.clone());
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(SdkError::JsonlNotFound {
                project_dir: project_dir.display().to_string(),
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            });
        }

        let sleep = tokio::time::sleep(poll_interval.min(remaining));
        tokio::pin!(sleep);

        tokio::select! {
            biased;
            res = state_rx.changed() => {
                if res.is_err() {
                    // Listener exited — fall through to fallback snapshot-diff
                    // until deadline.
                    continue;
                }
                // Loop back: re-check state for transcript_path.
            }
            () = &mut sleep => {
                // Resume: claude appends to the existing <session_id>.jsonl (no
                // new file), so snapshot-diff never fires. Target the known
                // transcript directly before falling back to snapshot-diff.
                if let Some(target) = &resume_target {
                    if tokio::fs::try_exists(target).await.unwrap_or(false) {
                        return Ok(target.clone());
                    }
                }
                // Fallback: snapshot-diff to catch the new JSONL even when
                // Stop hasn't fired yet (typical first-turn case).
                if let Some(path) = snapshot_diff_for_new(&project_dir, &before).await? {
                    return Ok(path);
                }
            }
        }
    }
}

async fn snapshot_diff_for_new(
    project_dir: &Path,
    before: &HashSet<String>,
) -> Result<Option<PathBuf>> {
    let current = list_jsonl_stems(project_dir).await?;
    let new_stems: Vec<String> = current.difference(before).cloned().collect();
    if new_stems.is_empty() {
        return Ok(None);
    }
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for stem in new_stems {
        let path = project_dir.join(format!("{stem}.jsonl"));
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if let Ok(mtime) = meta.modified() {
                match &best {
                    Some((_, t)) if *t > mtime => {}
                    _ => best = Some((path, mtime)),
                }
            }
        }
    }
    Ok(best.map(|(p, _)| p))
}

/// Exits on cancellation or when the FIFO reader closes.
///
/// Lives outside [`run_supervisor`] so [`super::query_tmux`] can await
/// `SessionStart` before pasting the first prompt, before the supervisor's
/// `TailTarget` is known.
pub(crate) fn spawn_hook_listener(
    hook_rx: mpsc::Receiver<HookLine>,
    cancellation_token: CancellationToken,
) -> (tokio::task::JoinHandle<()>, watch::Receiver<HookState>) {
    let (state_tx, state_rx) = watch::channel(HookState::Pending);
    let handle = tokio::spawn(async move {
        hook_listener(hook_rx, state_tx, cancellation_token).await;
    });
    (handle, state_rx)
}

/// Spawn the supervisor task that owns the tmux session, runs the tailer and
/// liveness monitor, and kills the session on cancellation.
///
/// `state_rx`, from [`spawn_hook_listener`], lets the supervisor read ongoing
/// `Stop` events, whose `transcript_path` overrides snapshot-diff discovery.
pub(crate) fn spawn_supervisor(
    session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
    state_rx: watch::Receiver<HookState>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_supervisor(session, target, tx, cancellation_token, state_rx).await;
    })
}

/// Lazy variant — used by `query_tmux_streaming` where we don't yet know the
/// session id (no prompt has been pasted yet).
///
/// The supervisor waits for the first `Stop` hook to report `transcript_path`,
/// falling back to snapshot-diff against `before`.
pub(crate) fn spawn_lazy_supervisor(
    session: Arc<TmuxSession>,
    project_dir: PathBuf,
    before: HashSet<String>,
    options: ClaudeOptions,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
    state_rx: watch::Receiver<HookState>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Honor options.init_timeout as an upper bound, but if the user never
        // pastes anything we just sit and wait until cancellation.
        let timeout = options.init_timeout.unwrap_or(Duration::from_hours(1));

        // Resume (`--resume <id>`) and `--continue` both append to an EXISTING
        // transcript, so snapshot-diff never sees a new file — target it
        // directly instead of waiting on the turn-end Stop hook. See
        // [`crate::transcripts::resolve_resume_target`].
        let resume_target = crate::transcripts::resolve_resume_target(&options, &project_dir).await;

        let discover_rx = state_rx.clone();
        let before_for_discover = before.clone();
        let discover = tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("lazy supervisor cancelled before JSONL discovery");
                session.kill().await;
                return;
            }
            res = discover_transcript_path(discover_rx, project_dir.clone(), before_for_discover, resume_target, timeout) => res,
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

        // Resume / --continue cases mean the JSONL already contains prior
        // turns. Seek to EOF before tailing — otherwise we'd replay every
        // old record and synth a Result for an old turn_duration.
        //
        // We can't rely on `before` here because Claude's project-dir
        // encoding doesn't always match ours (it replaces underscores in
        // path components too), so the file may exist on disk under a name
        // our snapshot-diff didn't see.
        let start_at_end = options.resume.is_some() || options.continue_conversation == Some(true);

        let target = TailTarget {
            path: jsonl_path,
            session_id,
            start_at_end,
        };
        run_supervisor(session, target, tx, cancellation_token, state_rx).await;
    })
}

async fn run_supervisor(
    session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
    state_rx: watch::Receiver<HookState>,
) {
    // Shared counter incremented by the tail_loop each time it forwards a
    // `SdkMessage::Result`. The stop_fallback_loop reads it to decide
    // whether the JSONL produced a Result for the current Stop event or
    // whether to synthesize one from the Stop payload.
    let results_emitted = Arc::new(AtomicU32::new(0));

    let session_id_hint = target.session_id.clone();

    // Sub-agent transcripts live beside the main one; the watcher tails them
    // and the main loop supplies the tool_use_id that threads them back on.
    let registry: SubagentRegistry =
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let subagent_task = target.path.parent().map(|project_dir| {
        spawn_subagent_watcher(
            subagent_dir(project_dir, &target.session_id),
            Arc::clone(&registry),
            tx.clone(),
            cancellation_token.clone(),
        )
    });

    let tailer_tx = tx.clone();
    let tailer_token = cancellation_token.clone();
    let tailer_session = Arc::clone(&session);
    let tailer_target = TailTarget {
        path: target.path.clone(),
        session_id: target.session_id.clone(),
        start_at_end: target.start_at_end,
    };
    let tailer_results = Arc::clone(&results_emitted);
    let tailer_registry = Arc::clone(&registry);
    let tailer_task = tokio::spawn(async move {
        tail_loop(
            tailer_session,
            tailer_target,
            tailer_tx,
            tailer_token,
            tailer_results,
            tailer_registry,
        )
        .await;
    });

    let liveness_tx = tx.clone();
    let liveness_token = cancellation_token.clone();
    let liveness_session = Arc::clone(&session);
    let liveness_task = tokio::spawn(async move {
        liveness_loop(liveness_session, liveness_tx, liveness_token).await;
    });

    let fallback_tx = tx.clone();
    let fallback_token = cancellation_token.clone();
    let fallback_results = Arc::clone(&results_emitted);
    let fallback_state_rx = state_rx.clone();
    let fallback_task = tokio::spawn(async move {
        stop_fallback_loop(
            fallback_state_rx,
            fallback_results,
            fallback_tx,
            session_id_hint,
            fallback_token,
        )
        .await;
    });

    // Wait for cancellation OR for the tailer + liveness tasks to exit.
    // The fallback task is detached — it exits on cancellation.
    tokio::select! {
        () = cancellation_token.cancelled() => {
            debug!("Supervisor: cancellation received");
        }
        () = async {
            let _ = tailer_task.await;
            let _ = liveness_task.await;
        } => {
            debug!("Supervisor: tailer + liveness tasks exited");
            cancellation_token.cancel();
        }
    }

    cancellation_token.cancel();
    let _ = fallback_task.await;
    if let Some(task) = subagent_task {
        let _ = task.await;
    }
    session.kill().await;
    info!(
        session_name = %session.session_name,
        "tmux supervisor exited and session killed"
    );
}

/// On each `Stop` hook event, wait up to ~2s for the tail loop to emit a
/// `Result` for that turn.
///
/// If the budget elapses without one, synthesize a `Result` from the Stop
/// payload's `last_assistant_message` (cost `0.0`) and forward it through `tx`.
async fn stop_fallback_loop(
    mut state_rx: watch::Receiver<HookState>,
    results_emitted: Arc<AtomicU32>,
    tx: mpsc::Sender<Result<SdkMessage>>,
    session_id_hint: String,
    cancellation_token: CancellationToken,
) {
    loop {
        let changed = tokio::select! {
            () = cancellation_token.cancelled() => return,
            r = state_rx.changed() => r,
        };
        if changed.is_err() {
            debug!("stop_fallback_loop: hook listener exited");
            return;
        }

        // Snapshot the latest state before borrowing it.
        let snapshot = state_rx.borrow().clone();
        let HookState::TurnEnded {
            last_assistant_message,
            session_id,
            ..
        } = snapshot
        else {
            continue;
        };

        let baseline = results_emitted.load(Ordering::SeqCst);

        let mut attempt: u32 = 0;
        let satisfied = loop {
            if cancellation_token.is_cancelled() {
                return;
            }
            if results_emitted.load(Ordering::SeqCst) > baseline {
                break true;
            }
            if attempt >= STOP_FALLBACK_RETRIES {
                break false;
            }
            tokio::select! {
                () = cancellation_token.cancelled() => return,
                () = tokio::time::sleep(STOP_FALLBACK_POLL) => {}
            }
            attempt += 1;
        };

        if satisfied {
            continue;
        }

        // Tail loop didn't emit. Try to synthesize from the Stop payload.
        let Some(last_text) = last_assistant_message else {
            debug!(
                "stop_fallback_loop: tail_loop didn't emit Result and payload \
                 has no last_assistant_message — giving up on this turn"
            );
            continue;
        };
        let payload_json = serde_json::json!({
            "last_assistant_message": last_text,
            "session_id": session_id.unwrap_or_else(|| session_id_hint.clone()),
        })
        .to_string();
        if let Some(msg) = translate_stop_payload_fallback(&payload_json, &session_id_hint) {
            debug!("stop_fallback_loop: emitting payload-derived Result");
            if tx.send(Ok(msg)).await.is_err() {
                debug!("stop_fallback_loop: consumer dropped");
                return;
            }
            // Count our own emission so a back-to-back Stop event doesn't
            // double-fire.
            results_emitted.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// Drain hook lines from the FIFO reader, parse payloads, and update the
/// `watch::Sender<HookState>` so consumers can react. Exits on cancellation or
/// when the upstream sender (the FIFO reader thread) closes.
async fn hook_listener(
    mut hook_rx: mpsc::Receiver<HookLine>,
    state_tx: watch::Sender<HookState>,
    cancellation_token: CancellationToken,
) {
    loop {
        let line = tokio::select! {
            () = cancellation_token.cancelled() => {
                debug!("hook_listener: cancellation received");
                return;
            }
            recv = hook_rx.recv() => {
                if let Some(l) = recv {
                    l
                } else {
                    debug!("hook_listener: FIFO sender closed");
                    return;
                }
            },
        };

        match line.event {
            HookEvent::SessionStart => {
                let session_id = extract_session_id(&line.payload);
                debug!(?session_id, "hook_listener: SessionStart");
                let _ = state_tx.send(HookState::SessionStarted { session_id });
            }
            HookEvent::Stop => {
                let transcript_path = extract_transcript_path(&line.payload).map(PathBuf::from);
                let last_assistant_message = extract_last_assistant_message(&line.payload);
                let session_id = extract_session_id(&line.payload);
                debug!(
                    transcript_path = ?transcript_path,
                    has_last_msg = last_assistant_message.is_some(),
                    "hook_listener: Stop"
                );
                let _ = state_tx.send(HookState::TurnEnded {
                    transcript_path,
                    last_assistant_message,
                    session_id,
                });
            }
            HookEvent::Unknown => {
                debug!(
                    payload_len = line.payload.len(),
                    "hook_listener: unknown event"
                );
            }
        }
    }
}

async fn tail_loop(
    _session: Arc<TmuxSession>,
    target: TailTarget,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
    results_emitted: Arc<AtomicU32>,
    registry: SubagentRegistry,
) {
    let path = target.path;
    let session_id = target.session_id;
    let start_at_end = target.start_at_end;
    let subagent_transcript = path
        .parent()
        .map_or_else(|| PathBuf::from("."), |dir| subagent_dir(dir, &session_id));

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

        // A finished Task call is the only place the sub-agent id and the
        // spawning tool_use_id appear together, and it carries the sub-agent's
        // totals — which no other record on this stream does.
        if let Some(done) = extract_subagent_completion(&value) {
            let transcript = subagent_transcript.join(format!("agent-{}.jsonl", done.agent_id));
            if let Some(usage) = sum_transcript_usage(&transcript).await {
                acc.accumulate_subagent_usage(&usage);
            } else {
                warn!(
                    agent_id = %done.agent_id,
                    path = %transcript.display(),
                    "No sub-agent usage found — turn total will omit it"
                );
            }
            registry
                .lock()
                .await
                .insert(done.agent_id.clone(), done.tool_use_id.clone());
            debug!(
                agent_id = %done.agent_id,
                tool_use_id = %done.tool_use_id,
                "Registered sub-agent completion"
            );
        }

        match translate_record(value, &mut acc) {
            Translated::Skip => {}
            Translated::One(msg) => {
                let is_result = matches!(*msg, SdkMessage::Result(_));
                if tx.send(Ok(*msg)).await.is_err() {
                    debug!("Consumer dropped — tailer exiting");
                    return;
                }
                if is_result {
                    results_emitted.fetch_add(1, Ordering::SeqCst);
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
            let _ = tx
                .send(Err(SdkError::TmuxSessionDied { pane_tail: None }))
                .await;
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
        permission_mode: crate::api::permissions::PermissionMode::BypassPermissions,
        cwd: options.cwd.clone().unwrap_or_default(),
        api_key_source: ApiKeySource::None,
        slash_commands: Vec::new(),
        output_style: crate::api::messages::OutputStyle::Normal,
        skills: Vec::new(),
        plugins: Vec::new(),
        agents: None,
        betas: options.betas.clone(),
        claude_code_version: String::new(),
    }
}

#[allow(dead_code)]
fn _path_unused(_p: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot-diff fallback is what the cluster relied on when the first
    /// turn outran the discovery timeout (the Stop hook had not fired yet). It
    /// must find a freshly-written transcript *in the polled `project_dir`*.
    #[tokio::test]
    async fn snapshot_diff_fallback_finds_new_transcript() {
        let base = std::env::temp_dir().join("tmux-discover-test-found");
        let _ = std::fs::remove_dir_all(&base);
        let project_dir = base.join("projects").join("-w-x");
        std::fs::create_dir_all(&project_dir).expect("mkdir");

        // No Stop hook → forces snapshot-diff against `project_dir`.
        let (_tx, state_rx) = watch::channel(HookState::Pending);
        let before = HashSet::new();

        // The transcript appears shortly after discovery starts, mirroring the
        // TUI writing it once the session is up.
        let jsonl = project_dir.join("724f6d06-deadbeef.jsonl");
        let writer = {
            let jsonl = jsonl.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(120)).await;
                std::fs::write(&jsonl, b"{\"type\":\"mode\"}\n").expect("write jsonl");
            })
        };

        let found =
            discover_transcript_path(state_rx, project_dir, before, None, Duration::from_secs(3))
                .await
                .expect("snapshot-diff should discover the new transcript");
        writer.await.expect("writer task");
        assert_eq!(found, jsonl);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Regression for the cold-config-dir bug: when the transcript lands in a
    /// DIFFERENT directory than the one discovery polls, discovery never sees
    /// it and fails with `JsonlNotFound`.
    ///
    /// The fix is in `resolve_project_dir`; this asserts the failure mode it
    /// prevents.
    #[tokio::test]
    async fn snapshot_diff_times_out_when_transcript_is_elsewhere() {
        let base = std::env::temp_dir().join("tmux-discover-test-elsewhere");
        let _ = std::fs::remove_dir_all(&base);
        let polled_dir = base.join("home-claude").join("projects").join("-w-x");
        let claude_writes = base.join("config-claude").join("projects").join("-w-x");
        std::fs::create_dir_all(&polled_dir).expect("mkdir polled");
        std::fs::create_dir_all(&claude_writes).expect("mkdir writes");
        // The transcript exists — but not where discovery is looking.
        std::fs::write(claude_writes.join("724f6d06-deadbeef.jsonl"), b"{}\n").expect("write");

        let (_tx, state_rx) = watch::channel(HookState::Pending);
        let before = HashSet::new();
        let err = discover_transcript_path(
            state_rx,
            polled_dir,
            before,
            None,
            Duration::from_millis(400),
        )
        .await
        .expect_err("discovery must fail when polling the wrong directory");
        assert!(matches!(err, SdkError::JsonlNotFound { .. }));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The resume case: `claude --resume <id>` appends to the existing
    /// `<id>.jsonl`, already in the pre-spawn snapshot, so snapshot-diff sees
    /// no new file and Stop may not fire in time.
    ///
    /// Targeting the known transcript directly must discover it quickly
    /// regardless.
    #[tokio::test]
    async fn discover_returns_existing_resume_target() {
        let base = std::env::temp_dir().join("tmux-discover-test-resume-target");
        let _ = std::fs::remove_dir_all(&base);
        let project_dir = base.join("projects").join("-w-x");
        std::fs::create_dir_all(&project_dir).expect("mkdir");

        // The transcript already exists (resume appends to it) and its stem is
        // in the pre-spawn snapshot — so snapshot-diff is blind to it.
        let session_id = "2bf52364-deadbeef";
        let jsonl = project_dir.join(format!("{session_id}.jsonl"));
        std::fs::write(&jsonl, b"{\"type\":\"mode\"}\n").expect("write jsonl");
        let mut before = HashSet::new();
        before.insert(session_id.to_string());

        // No Stop hook fires.
        let (_tx, state_rx) = watch::channel(HookState::Pending);

        let found = discover_transcript_path(
            state_rx,
            project_dir,
            before,
            Some(jsonl.clone()),
            Duration::from_secs(3),
        )
        .await
        .expect("resume target should be discovered directly");
        assert_eq!(found, jsonl);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// If the resume target never appears (wrong id / config dir didn't
    /// persist) and no new file shows up either, discovery must still time out
    /// with `JsonlNotFound` rather than hang — the `WithId` no-fallback
    /// contract.
    #[tokio::test]
    async fn resume_target_missing_falls_through_to_timeout() {
        let base = std::env::temp_dir().join("tmux-discover-test-resume-missing");
        let _ = std::fs::remove_dir_all(&base);
        let project_dir = base.join("projects").join("-w-x");
        std::fs::create_dir_all(&project_dir).expect("mkdir");

        let missing = project_dir.join("does-not-exist.jsonl");
        let (_tx, state_rx) = watch::channel(HookState::Pending);
        let before = HashSet::new();

        let err = discover_transcript_path(
            state_rx,
            project_dir,
            before,
            Some(missing),
            Duration::from_millis(400),
        )
        .await
        .expect_err("discovery must time out when the resume target never exists");
        assert!(matches!(err, SdkError::JsonlNotFound { .. }));

        let _ = std::fs::remove_dir_all(&base);
    }
}

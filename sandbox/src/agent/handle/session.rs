//! Starting and fingerprinting CLI sessions: agent-type normalization, resume
//! resolution, the persistent-tmux session fingerprint, and `start_session`.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_events::{AgentSubType, TaskTurnSubType};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::agent_state::AgentState;
use crate::agent::codex_exec;
use crate::agent::options::build_options;
use crate::agent::types::{AgentRunRequest, ResumeSession};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::events::GrpcEventPublisher;

use super::claude::make_user_message;
use super::retry::is_session_unavailable_error;
use super::{
    ActiveHandle, AgentRuntimeError, QueuedSend, AGENT_TYPE_CLAUDE_CODE,
    AGENT_TYPE_CLAUDE_CODE_FABLE, AGENT_TYPE_CLAUDE_CODE_TMUX, AGENT_TYPE_CODEX,
};

pub(super) fn normalize_agent_type(agent_type: Option<&str>) -> &'static str {
    match agent_type {
        Some(AGENT_TYPE_CODEX) => AGENT_TYPE_CODEX,
        Some(AGENT_TYPE_CLAUDE_CODE)
        | Some(AGENT_TYPE_CLAUDE_CODE_TMUX)
        | Some(AGENT_TYPE_CLAUDE_CODE_FABLE)
        | None => AGENT_TYPE_CLAUDE_CODE,
        Some(other) => {
            warn!(
                agent_type = other,
                "Unknown agent_type, defaulting to claude-code"
            );
            AGENT_TYPE_CLAUDE_CODE
        }
    }
}

/// Resolve the `ResumeSession` for a new agent run.
///
/// Order of precedence:
/// 1. A non-empty `previous_session_id` (carried from the live agent state).
/// 2. A non-empty `resume_session_id` from the backend / recovery path.
/// 3. For `claude-code`, fall back to `Last` (the CLI's `--continue`) so a pod
///    that lost track of the session ID still resumes the on-disk JSONL state.
/// 4. Otherwise no resume.
///
/// Empty strings are treated as "no resume" to avoid producing `--resume ""`,
/// which the Claude CLI rejects (transport closes immediately on init).
fn resolve_resume_session(
    previous_session_id: &str,
    resume_session_id: Option<&str>,
    agent_type: &str,
) -> Option<ResumeSession> {
    if !previous_session_id.is_empty() {
        return Some(ResumeSession::WithId(previous_session_id.to_string()));
    }
    if let Some(session_id) = resume_session_id.filter(|s| !s.is_empty()) {
        info!(resume_session_id = %session_id, "Using backend-provided session ID for resume");
        return Some(ResumeSession::WithId(session_id.to_string()));
    }
    if agent_type == AGENT_TYPE_CLAUDE_CODE {
        Some(ResumeSession::Last)
    } else {
        None
    }
}

/// Wrap a prompt with instructions to recover lost conversation context via
/// the moat CLI. Used by the fresh-session fallback: the new session starts
/// without the prior conversation in context, but the full history is still
/// readable through `moat messages list` (the `moat` binary ships in the
/// sandbox image and authenticates automatically).
fn context_recovery_prompt(task_id: &str, prompt: &str) -> String {
    format!(
        "<context-recovery>\n\
         Your previous session for this task could not be resumed, so the prior \
         conversation is not in your context. Before acting, recover it by running:\n\
         \x20 moat messages list {task_id} --full --limit 100\n\
         (if it reports older messages, rerun with the suggested --before-id to page back)\n\
         Treat that transcript as the conversation so far and continue from it.\n\
         </context-recovery>\n\n\
         {prompt}"
    )
}

/// A successfully started CLI session plus the metadata the actor needs to
/// manage it (tmux persistent-session fingerprinting and turn cancellation).
pub(super) struct StartedSession {
    pub(super) handle: ActiveHandle,
    /// Session-level cancel token (also wired into `ClaudeOptions`). Reused
    /// by `set_running` for later pasted / CLI-internal turns in tmux mode.
    pub(super) cancel_token: CancellationToken,
    /// The append-system-prompt the session was started with (fingerprint).
    pub(super) system_prompt: Option<String>,
}

/// Fingerprint + control state of the live persistent tmux session.
///
/// In tmux mode the Claude TUI survives turn ends so the CLI's internal
/// `ScheduleWakeup` timer and `Monitor` watcher keep running. A new send is
/// pasted into the live session only when it is compatible with this
/// fingerprint — otherwise the session is killed and respawned with
/// `--resume`.
pub(super) struct TmuxSessionInfo {
    pub(super) model: Option<String>,
    /// Normalized agent type (see [`normalize_agent_type`]).
    pub(super) agent_type: &'static str,
    pub(super) agent_mode: Option<String>,
    pub(super) system_prompt: Option<String>,
    pub(super) cancel_token: CancellationToken,
}

impl TmuxSessionInfo {
    /// True when `send` can't run inside this session and a restart
    /// (kill + respawn with `--resume`) is required.
    pub(super) fn needs_restart(
        &self,
        send: &QueuedSend,
        fresh_system_prompt: Option<&str>,
        current_session_id: &str,
    ) -> bool {
        if normalize_agent_type(send.agent_type.as_deref()) != self.agent_type {
            return true;
        }
        if send.model != self.model {
            return true;
        }
        if send.agent_mode != self.agent_mode {
            return true;
        }
        // Backend asked to resume a different session (e.g. recovery after
        // a pod restart raced an old live session).
        if let Some(resume_id) = send
            .resume_session_id
            .as_deref()
            .filter(|id| !id.is_empty())
        {
            if resume_id != current_session_id {
                return true;
            }
        }
        // A changed system prompt (repo/channel prompt edited mid-task) only
        // applies to a fresh CLI process.
        fresh_system_prompt != self.system_prompt.as_deref()
    }
}

/// Publish the turn-start events for a turn that begins without a
/// `start_session` call: a message pasted into the live persistent tmux
/// session, or a turn the CLI started on its own (internal `ScheduleWakeup`
/// fired / `Monitor` event). Mirrors the events published by `start_session`.
pub(super) async fn publish_turn_start(events: &GrpcEventPublisher, task_id: &str) {
    events
        .publish_agent_event(task_id, AgentSubType::Started, serde_json::json!({}))
        .await;
    events
        .publish_task_turn_event(task_id, TaskTurnSubType::Active, serde_json::json!({}))
        .await;
}

/// Fetch the assembled append-system-prompt from the backend. On failure,
/// fall back to whatever was last cached in this pod. Cache is purely a
/// resilience layer — every successful fetch refreshes it.
pub(super) async fn fetch_system_prompt(
    backend_client: &dyn BackendClient,
    task_id: &str,
    system_prompt_cache: &RwLock<Option<String>>,
) -> Option<String> {
    match backend_client.get_system_prompt(task_id).await {
        Ok(prompt) => {
            *system_prompt_cache.write().await = Some(prompt.clone());
            Some(prompt)
        }
        Err(error) => {
            let cached = system_prompt_cache.read().await.clone();
            warn!(
                error = %error,
                has_cache = cached.is_some(),
                "Failed to fetch system prompt; using cached fallback"
            );
            cached
        }
    }
}

/// Start a new CLI session.
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor later
pub(super) async fn start_session(
    prompt: &str,
    config: &Config,
    agent_state: Arc<AgentState>,
    events: &GrpcEventPublisher,
    credentials: &RwLock<HashMap<String, String>>,
    backend_client: &dyn BackendClient,
    task_id: &str,
    previous_session_id: &str,
    model: Option<&str>,
    agent_type: Option<&str>,
    agent_mode: Option<&str>,
    resume_session_id: Option<String>,
    credential_failures: CredentialFailureDetector,
    system_prompt_cache: Arc<RwLock<Option<String>>>,
) -> Result<StartedSession, AgentRuntimeError> {
    let system_prompt = fetch_system_prompt(backend_client, task_id, &system_prompt_cache).await;

    match backend_client.get_credentials(task_id).await {
        Ok(resp) => {
            // Refresh mode: keep a locally-rotated `~/.codex/auth.json`. The
            // codex_watcher pushes rotations back to the backend; rewriting
            // here from the (pre-rotation) backend copy would clobber them
            // and trigger `refresh_token_reused` on the next codex call.
            crate::init::credentials::persist_agent_credentials(
                config,
                &resp.credentials,
                crate::init::credentials::PersistMode::Refresh,
            )
            .await;
            // Filter out git-provider tokens before passing to the agent subprocess env.
            // Git/gh ops inside the agent read tokens fresh from `~/.config/gh/hosts.yml`.
            *credentials.write().await =
                crate::init::credentials::filter_agent_credentials(&resp.credentials);
        }
        Err(error) => {
            warn!(error = %error, "Failed to refresh credentials, using cached");
        }
    }

    let creds = credentials.read().await;
    let env = if creds.is_empty() {
        None
    } else {
        Some(creds.clone())
    };
    drop(creds);

    let agent_type = normalize_agent_type(agent_type);

    let resume = resolve_resume_session(
        previous_session_id,
        resume_session_id.as_deref(),
        agent_type,
    );

    let request = AgentRunRequest {
        content: Vec::new(),
        model: model.map(std::string::ToString::to_string),
        system_prompt: system_prompt.clone(),
        resume,
        agent_type: Some(agent_type.to_string()),
        agent_mode: agent_mode.map(std::string::ToString::to_string),
    };

    let cancel_token = CancellationToken::new();
    agent_state.set_running(cancel_token.clone()).await;

    events
        .publish_agent_event(task_id, AgentSubType::Started, serde_json::json!({}))
        .await;
    events
        .publish_task_turn_event(task_id, TaskTurnSubType::Active, serde_json::json!({}))
        .await;

    info!(
        task_id = %task_id,
        resume = ?request.resume,
        agent_type,
        use_tmux = config.use_tmux,
        "Starting agent session"
    );

    if agent_type == AGENT_TYPE_CODEX {
        return codex_exec::start(prompt, config, &request, env, credential_failures)
            .await
            .map(|handle| StartedSession {
                handle: ActiveHandle::Codex(handle),
                cancel_token,
                system_prompt,
            })
            .map_err(|error| {
                AgentRuntimeError::Codex(format!("Failed to start Codex CLI: {error}"))
            });
    }

    let options = build_options(
        config,
        &request,
        cancel_token.clone(),
        Arc::clone(&agent_state),
        env.clone(),
    );

    let sdk_query = |p: &str, o: claude_agent_sdk::ClaudeOptions| {
        let use_tmux = config.use_tmux;
        let prompt = p.to_owned();
        let paste_session_id = previous_session_id.to_owned();
        async move {
            if use_tmux {
                // Persistent session: spawn the TUI without a prompt, then
                // paste the first message into it. The session survives turn
                // ends so the CLI's internal ScheduleWakeup timer and Monitor
                // watcher keep running between turns.
                let handle = claude_agent_sdk::query_tmux_streaming(o).await?;
                let user_msg = make_user_message(&prompt, &paste_session_id);
                if let Err(error) = handle.send_user_message(&user_msg).await {
                    handle.close();
                    return Err(error);
                }
                Ok(handle)
            } else {
                claude_agent_sdk::query(&prompt, o).await
            }
        }
    };

    match sdk_query(prompt, options).await {
        Ok(handle) => Ok(StartedSession {
            handle: ActiveHandle::Claude(handle),
            cancel_token,
            system_prompt,
        }),
        // Fall back to a fresh session when resume can't load its target:
        // - `--continue` (ResumeSession::Last): any startup error, as before.
        // - `--resume <id>` (ResumeSession::WithId): only when the error says
        //   the session transcript is unavailable (e.g. stale session id after
        //   a pod restart) — see [`is_session_unavailable_error`]. Without this
        //   arm a stale id was a terminal agent error and the user's message
        //   sat unacknowledged until the next pod restart.
        Err(error)
            if request.resume == Some(ResumeSession::Last)
                || (request.resume.is_some() && is_session_unavailable_error(&error)) =>
        {
            let stale_session_id = match &request.resume {
                Some(ResumeSession::WithId(id)) => id.clone(),
                _ => String::new(),
            };
            warn!(
                error = %error,
                stale_session_id = %stale_session_id,
                "Resume failed, starting fresh session (previous context lost)"
            );
            // Surface the context loss: the backend persists this as an inline
            // conversation notice so the user sees it in the message history.
            events
                .publish_agent_event(
                    task_id,
                    AgentSubType::SessionContextLost,
                    serde_json::json!({
                        "stale_session_id": stale_session_id,
                        "error": error.to_string(),
                        "reason": "transcript_unavailable",
                    }),
                )
                .await;
            let fresh_request = AgentRunRequest {
                resume: None,
                ..request
            };
            let fresh_options = build_options(
                config,
                &fresh_request,
                cancel_token.clone(),
                agent_state,
                env,
            );
            // The fresh session starts without the prior conversation —
            // instruct the agent to recover it via the moat CLI first.
            sdk_query(&context_recovery_prompt(task_id, prompt), fresh_options)
                .await
                .map(|handle| StartedSession {
                    handle: ActiveHandle::Claude(handle),
                    cancel_token,
                    system_prompt,
                })
                .map_err(AgentRuntimeError::from)
        }
        Err(error) => Err(AgentRuntimeError::from(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[test]
    fn context_recovery_prompt_wraps_original_prompt() {
        let out = context_recovery_prompt("task-abc", "fix the login bug");
        assert!(
            out.contains("moat messages list task-abc --full --limit 100"),
            "must reference the moat CLI recovery command: {out}"
        );
        assert!(out.starts_with("<context-recovery>"));
        assert!(
            out.ends_with("fix the login bug"),
            "original prompt must come last, unmodified: {out}"
        );
    }

    #[test]
    fn normalize_unknown_agent_type_defaults_to_claude() {
        assert_eq!(
            normalize_agent_type(Some("something-else")),
            AGENT_TYPE_CLAUDE_CODE
        );
    }

    #[test]
    fn normalize_claude_code_tmux_is_claude_family() {
        assert_eq!(
            normalize_agent_type(Some(AGENT_TYPE_CLAUDE_CODE_TMUX)),
            AGENT_TYPE_CLAUDE_CODE
        );
    }

    // ── TmuxSessionInfo::needs_restart ──────────────────────────────

    fn test_tmux_session_info() -> TmuxSessionInfo {
        TmuxSessionInfo {
            model: Some("claude-sonnet-4-6".to_string()),
            agent_type: AGENT_TYPE_CLAUDE_CODE,
            agent_mode: None,
            system_prompt: Some("base prompt".to_string()),
            cancel_token: CancellationToken::new(),
        }
    }

    fn test_queued_send() -> QueuedSend {
        let (ack, _rx) = oneshot::channel();
        QueuedSend {
            text: "hello".to_string(),
            model: Some("claude-sonnet-4-6".to_string()),
            agent_type: Some("claude-code".to_string()),
            agent_mode: None,
            resume_session_id: None,
            ack,
        }
    }

    #[test]
    fn needs_restart_false_for_compatible_send() {
        let info = test_tmux_session_info();
        let send = test_queued_send();
        assert!(!info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_true_on_model_change() {
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.model = Some("claude-opus-4-8".to_string());
        assert!(info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_true_on_agent_mode_change() {
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.agent_mode = Some("plan".to_string());
        assert!(info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_true_on_agent_type_change() {
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.agent_type = Some("codex".to_string());
        assert!(info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_false_for_agent_type_alias() {
        // claude-code-tmux / claude-code-fable normalize to the claude-code
        // family — not a session-incompatible change.
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.agent_type = Some("claude-code-tmux".to_string());
        assert!(!info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_true_on_system_prompt_change() {
        let info = test_tmux_session_info();
        let send = test_queued_send();
        assert!(info.needs_restart(&send, Some("edited prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_true_on_foreign_resume_session_id() {
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.resume_session_id = Some("other-session".to_string());
        assert!(info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    #[test]
    fn needs_restart_false_when_resume_session_id_matches_current() {
        let info = test_tmux_session_info();
        let mut send = test_queued_send();
        send.resume_session_id = Some("session-1".to_string());
        assert!(!info.needs_restart(&send, Some("base prompt"), "session-1"));
    }

    // ── resolve_resume_session ──────────────────────────────────────

    #[test]
    fn resolve_resume_skips_empty_backend_session_for_claude() {
        // Recovery path with an empty agent_session_id (the bug that wedged
        // production task 75ab270a-…). Must fall through to Last, not WithId("").
        let resume = resolve_resume_session("", Some(""), AGENT_TYPE_CLAUDE_CODE);
        assert_eq!(resume, Some(ResumeSession::Last));
    }

    #[test]
    fn resolve_resume_skips_empty_backend_session_for_codex() {
        let resume = resolve_resume_session("", Some(""), AGENT_TYPE_CODEX);
        assert_eq!(resume, None);
    }

    #[test]
    fn resolve_resume_uses_previous_session_id_when_present() {
        let resume = resolve_resume_session("prev-abc", Some(""), AGENT_TYPE_CLAUDE_CODE);
        assert_eq!(resume, Some(ResumeSession::WithId("prev-abc".to_string())));
    }

    #[test]
    fn resolve_resume_uses_backend_session_id_when_non_empty() {
        let resume = resolve_resume_session("", Some("sess-123"), AGENT_TYPE_CLAUDE_CODE);
        assert_eq!(resume, Some(ResumeSession::WithId("sess-123".to_string())));
    }

    #[test]
    fn resolve_resume_falls_back_to_last_for_claude_with_no_input() {
        let resume = resolve_resume_session("", None, AGENT_TYPE_CLAUDE_CODE);
        assert_eq!(resume, Some(ResumeSession::Last));
    }

    #[test]
    fn resolve_resume_returns_none_for_codex_with_no_input() {
        let resume = resolve_resume_session("", None, AGENT_TYPE_CODEX);
        assert_eq!(resume, None);
    }
}

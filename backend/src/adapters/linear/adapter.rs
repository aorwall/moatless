//! Linear adapter — Agents trait impl. The webhook dispatcher lives in
//! `handlers.rs`; this module owns the [`Adapter`] trait surface (outbound
//! `agentActivityCreate` calls) and the workspace-scoped configuration
//! (webhook secret, OAuth client credentials, dedup cache).

use std::time::Duration;

use moka::future::Cache;
use tracing::warn;

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, ExternalStatus, InboundMessage, LoopContext, OutboundReply,
    SubscriptionMatch, TaskBinding,
};
use crate::adapters::{Adapter, AdapterKind};
use crate::db::DbPool;

use super::agent_api::{default_http_client, LinearAgentApi};
use super::status;
use super::SUBJECT_KIND_ISSUE;

/// Linear adapter.
pub struct LinearAdapter {
    /// Global webhook secret from env (`LINEAR_CHANNEL_WEBHOOK_SECRET`).
    /// Used as a fallback when the connection has no `webhook_secret` set.
    pub(crate) global_webhook_secret: Option<String>,
    /// OAuth client_id used by the Linear Agents API client to refresh
    /// workspace tokens. `None` disables the OAuth refresh path.
    oauth_client_id: Option<String>,
    /// OAuth client_secret used by the Linear Agents API client for refresh.
    oauth_client_secret: Option<String>,
    /// Database pool for looking up workspace tokens.
    db: DbPool,
    /// Deduplication cache with a 5-minute TTL, keyed on a per-event resource
    /// identity (`{event_type}:{action}:{resource_id}` — see
    /// `handlers::dedup_key`), NOT Linear's stable `webhookId`. This collapses
    /// genuine Linear retries while never dropping distinct events. Per-pod and
    /// in-memory, so cross-pod retries can still pass through — the
    /// session-binding guard in `provision_session_task` covers the
    /// resulting duplicate-task risk.
    dedup_cache: Cache<String, ()>,
}

/// Build an outbound Linear comment body: the reply text followed by the
/// optional `Moatless task:` footer linking back to the moatless task. The
/// footer URL doubles as the echo-filter source — when the comment's own
/// webhook echo arrives, the authoring task id is recovered from that link
/// (see `agent_runtime::handle_comment_created`).
fn build_comment_body(message: &OutboundReply) -> String {
    let mut body = message.text.clone();
    if let Some(ref footer) = message.footer {
        body.push_str(footer);
    }
    body
}

impl LinearAdapter {
    pub fn new(
        global_webhook_secret: Option<String>,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
        db: DbPool,
    ) -> Self {
        Self {
            global_webhook_secret,
            oauth_client_id,
            oauth_client_secret,
            db,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_mins(5))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Build a Linear Agents API client for the given workspace.
    pub(crate) fn agent_api_for_org(&self, org_id: &str) -> LinearAgentApi {
        LinearAgentApi::new(
            default_http_client(),
            self.db.clone(),
            org_id.to_string(),
            self.oauth_client_id.clone(),
            self.oauth_client_secret.clone(),
        )
    }

    /// Build a [`LinearAgentApi`] for the single Linear connection in
    /// `integration_connections`. Used by [`Adapter::post_to_destination`],
    /// which takes no connection parameter. Errors if there is no Linear
    /// connection installed, or more than one — multi-org callers must go
    /// through the typed `/api/v1/linear/*` endpoints with an explicit
    /// `connectionId`.
    async fn api_for_single_workspace(&self) -> Result<LinearAgentApi, AdapterError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT scope_value FROM integration_connections \
             WHERE provider = 'linear' LIMIT 2",
        )
        .fetch_all(&self.db)
        .await
        .map_err(AdapterError::Db)?;
        match rows.len() {
            0 => Err(AdapterError::AdapterNotConfigured(
                "No Linear workspace installed — run the Linear OAuth install first".into(),
            )),
            1 => Ok(self.agent_api_for_org(&rows[0])),
            _ => Err(AdapterError::BadRequest(
                "Multiple Linear workspaces installed — use the /api/v1/linear/* \
                 endpoints with an explicit connectionId"
                    .into(),
            )),
        }
    }

    /// Exposed accessor for the new dispatcher in `handlers.rs`.
    pub(crate) fn global_webhook_secret_for_dispatch(&self) -> Option<&str> {
        self.global_webhook_secret.as_deref()
    }

    /// Exposed wrapper for the new dispatcher in `handlers.rs`. `key` is the
    /// per-event dedup identity from `handlers::dedup_key`.
    pub(crate) async fn mark_seen_for_dispatch(&self, key: &str) -> bool {
        self.mark_seen(key).await
    }

    /// Mark a dedup `key` as processed; returns `true` if this is the first
    /// time we see it (caller should proceed), or `false` if it's a dup.
    pub(crate) async fn mark_seen(&self, key: &str) -> bool {
        if self.dedup_cache.get(key).await.is_some() {
            return false;
        }
        self.dedup_cache.insert(key.to_string(), ()).await;
        true
    }
}

#[async_trait::async_trait]
impl Adapter for LinearAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Linear
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Linear comments allow generous bodies; cap well below the documented
    /// 50k limit to leave headroom for the `Moatless task:` footer.
    fn max_message_length(&self) -> usize {
        50_000
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        let user_id = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM provider_tokens WHERE provider = 'linear' AND provider_login = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(AdapterError::Db)?;
        Ok(user_id)
    }

    async fn send_reply(
        &self,
        _thread: &BindingContext,
        _config: &LoopContext,
        _message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        Err(AdapterError::BadRequest(
            "Linear outbound now goes through send_reply_for_binding (Agents API)".into(),
        ))
    }

    /// Post a comment on a Linear issue. Destination format:
    /// `issue:<identifier-or-uuid>` (e.g. `issue:SP-123` or
    /// `issue:bf0e…`). The single Linear connection in `integration_connections`
    /// is used as the workspace context — multi-org callers must use the
    /// typed `/api/v1/linear/*` endpoints instead.
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<crate::adapters::DestinationPostResult, AdapterError> {
        let id_or_key = destination.strip_prefix("issue:").unwrap_or(destination);
        if id_or_key.is_empty() {
            return Err(AdapterError::BadRequest(
                "Linear destination must be `issue:<SP-N>` or `issue:<uuid>`".into(),
            ));
        }

        let api = self.api_for_single_workspace().await?;
        let issue = api.get_issue(id_or_key).await?;
        let issue_uuid = issue
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AdapterError::AdapterApi("Linear issue payload missing id".into()))?
            .to_string();
        let identifier = issue
            .get("identifier")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(id_or_key)
            .to_string();
        let url = issue
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);

        let comment = api.create_comment(&issue_uuid, &message.text).await?;
        let comment_id = comment
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::AdapterApi("Linear commentCreate missing comment.id".into())
            })?
            .to_string();

        let mut destination_config = serde_json::json!({
            "issue_id": issue_uuid,
            "issue_identifier": identifier,
        });
        if let Some(u) = url {
            destination_config["url"] = serde_json::Value::String(u);
        }

        Ok(crate::adapters::DestinationPostResult {
            message_external_id: format!("linear:comment:{comment_id}"),
            subject_kind: SUBJECT_KIND_ISSUE,
            subject_key: issue_uuid,
            destination_config,
        })
    }

    /// Reply to a bound Linear destination. Routes by `subject_kind`:
    /// - `linear_agent_session` → `commentCreate` on the session's bound issue
    /// - `linear_issue` → `commentCreate` (CLI-created or operator-bound issue)
    ///
    /// The terminal `response` activity that closes the Linear agent session is
    /// no longer emitted here — it is posted by `handle_agent_completed` once
    /// the task finishes, using the last assistant message body. This keeps
    /// the agent-session lifecycle owned by a single emission point.
    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        _rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let api = self.agent_api_for_org(&connection.scope_value);

        if binding.subject_kind == SUBJECT_KIND_ISSUE {
            let issue_uuid = binding
                .destination_config
                .get("issue_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&binding.subject_key);
            let body = build_comment_body(message);
            let comment = api.create_comment(issue_uuid, &body).await?;
            let comment_id = comment
                .get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    AdapterError::AdapterApi("Linear commentCreate missing comment.id".into())
                })?;
            return Ok(Some(format!("linear:comment:{comment_id}")));
        }

        // AgentSession binding: post the reply as a comment on the bound
        // Linear issue (where conversation actually happens). The session is
        // finalized later by `handle_agent_completed` posting a `response`
        // activity with the last assistant message body.
        let session_id = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest(
                    "Linear binding missing session_id in destination_config".into(),
                )
            })?;

        let issue_uuid = binding
            .destination_config
            .get("issue_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest(
                    "Linear binding missing issue_id in destination_config".into(),
                )
            })?;

        let body = build_comment_body(message);
        match api.create_comment(issue_uuid, &body).await {
            Ok(comment) => {
                let comment_id = comment
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Ok(comment_id.map(|cid| format!("linear:comment:{cid}")))
            }
            Err(e) => {
                warn!(
                    issue_id = issue_uuid,
                    session_id,
                    error = %e,
                    "Linear issue comment failed; the session will still be finalized on agent.completed"
                );
                Ok(None)
            }
        }
    }

    fn reply_instructions(&self) -> Option<String> {
        Some(
            "## Linear Agent Notes\n\
             - Intermediate progress (text + tool calls) is emitted automatically \
               as ephemeral `thought` / `action` activities — you do not need to \
               post status updates by hand.\n\
             - Your final reply is posted as a comment on the Linear issue. The \
               agent session is finalized automatically when the task completes, \
               by emitting a `response` activity carrying your last assistant \
               message — you do not need to do this by hand. Use \
               `moat messages reply` once to post.\n\
             - Markdown is supported. Linear URLs auto-render as mentions.\n\
             - The Moatless task URL is already linked via the session's \
               external URLs — no need to include it in the reply body."
                .to_string(),
        )
    }

    fn thread_reply_hint(&self, _msg: &InboundMessage) -> Option<String> {
        Some("[Linear Agent prompt]".to_string())
    }

    fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
        status::binding_status_key(binding)
    }

    fn is_terminal_state_kind(&self, state_kind: &str) -> bool {
        status::is_terminal_state_kind(state_kind)
    }

    /// Linear workspace tokens are per-org, not per-user, so `acting_user` is
    /// unused here.
    async fn fetch_binding_status(
        &self,
        binding: &TaskBinding,
        _acting_user: &str,
    ) -> Option<ExternalStatus> {
        let (issue_id, org_id) = status::binding_subject(binding)?;

        match self.agent_api_for_org(org_id).get_issue(issue_id).await {
            Ok(issue) => Some(status::status_from_issue(&issue)),
            Err(e) => {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to fetch live Linear issue status"
                );
                None
            }
        }
    }

    // `add_reaction` / `remove_reaction` intentionally not implemented; the
    // default trait impl returns `BadRequest`, which is the new behavior.
    // `fetch_context` is similarly defaulted (Linear Agents already deliver
    // the prompt context inline via `promptContext` in the webhook).
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::footer::parse_task_id_from_body;

    fn reply(text: &str, footer: Option<&str>) -> OutboundReply {
        OutboundReply {
            text: text.to_string(),
            task_url: None,
            footer: footer.map(str::to_string),
            author_user_id: None,
            is_error: false,
            files: vec![],
        }
    }

    #[test]
    fn comment_body_carries_text_and_footer_without_hidden_marker() {
        let footer = "\n\n---\nMoatless task: [T](https://app.example.com/tasks/task-42)";
        let body = build_comment_body(&reply("hello", Some(footer)));
        // No hidden HTML marker — the body is plain text + the visible footer.
        assert!(!body.contains("<!-- moatless-agent"));
        assert!(body.starts_with("hello"));
        assert!(body.ends_with(footer));
        // The authoring task id is recoverable from the footer URL on echo-back.
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", &body),
            Some("task-42".to_string())
        );
    }

    #[test]
    fn comment_body_without_footer_is_just_text() {
        let body = build_comment_body(&reply("hi", None));
        assert_eq!(body, "hi");
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", &body),
            None
        );
    }
}

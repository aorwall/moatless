//! Linear adapter — Agents trait impl. The webhook dispatcher lives in
//! `handlers.rs`; this module owns the [`Adapter`] trait surface (outbound
//! `agentActivityCreate` calls) and the workspace-scoped configuration
//! (webhook secret, OAuth client credentials).

use std::sync::Arc;

use tracing::warn;

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, ExternalStatus, InboundMessage, LoopContext, OutboundReply,
    SubscriptionMatch, TaskBinding,
};
use crate::adapters::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDirectDestination,
    AdapterInboundPresentation, AdapterKind, AdapterMessageReply, AdapterOutboundContent,
    AdapterPostBindingFollowup,
};
use crate::db::DbPool;
use crate::state::AppState;

use super::agent_api::LinearAgentApi;
use super::auth::LinearCredentials;
use super::status;
use super::SUBJECT_KIND_ISSUE;

/// Linear adapter.
pub struct LinearAdapter {
    /// Global webhook secret from env (`LINEAR_CHANNEL_WEBHOOK_SECRET`).
    /// Used as a fallback when the connection has no `webhook_secret` set.
    pub(crate) global_webhook_secret: Option<String>,
    /// How this Adapter's outbound calls authenticate, and which host they
    /// reach. Owned by [`auth`](super::auth), which is the only module that
    /// reads them.
    credentials: LinearCredentials,
    /// Database pool for looking up workspace tokens.
    db: DbPool,
    /// Runtime config used to decrypt DB-backed Adapter app secrets.
    config: Option<Arc<crate::config::Config>>,
}

/// Build an outbound Linear comment body: the reply text followed by the
/// optional `Moatless task:` footer linking back to the moatless task. The
/// footer URL doubles as the echo-filter source — when the comment's own
/// webhook echo arrives, the authoring task id is recovered from that link
/// (see `comments::handle_comment_created`).
fn build_comment_body(message: &OutboundReply) -> String {
    let mut body = message.text.clone();
    if let Some(ref footer) = message.footer {
        body.push_str(footer);
    }
    body
}

impl LinearAdapter {
    /// Build the Adapter from the one place its credentials come from.
    ///
    /// Taking [`LinearConfig`] rather than three loose `Option<String>`s is
    /// what makes the Adapter's credentials provably the same ones the
    /// `AppState`-side paths use — the two used to be assembled separately,
    /// and nothing checked they agreed. It is also the injection seam: a test
    /// builds a `LinearConfig` with `api_base_url` pointed at a loopback stub
    /// and gets an Adapter whose outbound calls never leave the machine,
    /// through the same constructor production calls.
    pub fn new(config: &crate::config::LinearConfig, db: DbPool) -> Self {
        Self {
            global_webhook_secret: config.webhook_secret.clone(),
            credentials: LinearCredentials::from(config),
            db,
            config: None,
        }
    }

    pub fn from_db(config: Arc<crate::config::Config>, db: DbPool) -> Self {
        Self {
            global_webhook_secret: config.linear.webhook_secret.clone(),
            credentials: LinearCredentials::from(&config.linear),
            db,
            config: Some(config),
        }
    }

    /// Build a Linear Agents API client for the given workspace.
    pub(crate) fn agent_api_for_org(&self, org_id: &str) -> LinearAgentApi {
        self.credentials.api_for_org(self.db.clone(), org_id)
    }

    pub(crate) async fn agent_api_for_connection(
        &self,
        connection: &AdapterConnection,
    ) -> Result<LinearAgentApi, AdapterError> {
        self.agent_api_for_transport(&connection.scope_value, &connection.transport_config)
            .await
    }

    pub(crate) async fn agent_api_for_transport(
        &self,
        org_id: &str,
        transport_config: &serde_json::Value,
    ) -> Result<LinearAgentApi, AdapterError> {
        let Some(config) = self.config.as_deref() else {
            return Ok(self.agent_api_for_org(org_id));
        };
        let app_key = crate::adapter_apps::app_key_from_transport_config(transport_config);
        let Some(app) = crate::adapter_apps::linear_app_config(&self.db, config, app_key)
            .await
            .map_err(|e| AdapterError::AdapterApi(e.to_string()))?
        else {
            return Err(AdapterError::AdapterNotConfigured(format!(
                "linear app '{app_key}'"
            )));
        };
        Ok(LinearCredentials::from(&app).api_for_org(self.db.clone(), org_id))
    }

    /// The pool [`auth`](super::auth) reads workspace rows from. Exposed to the
    /// sibling module rather than the query being written here — the Adapter
    /// type owns construction and identity, not database access.
    pub(super) fn db(&self) -> &DbPool {
        &self.db
    }

    /// Exposed accessor for the dispatcher in `handlers.rs`.
    pub(crate) fn global_webhook_secret_for_dispatch(&self) -> Option<&str> {
        self.global_webhook_secret.as_deref()
    }
}

impl Adapter for LinearAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Linear
    }
}

impl AdapterOutboundContent for LinearAdapter {
    /// Linear comments allow generous bodies; cap well below the documented
    /// 50k limit to leave headroom for the `Moatless task:` footer.
    fn max_message_length(&self) -> usize {
        50_000
    }
}

#[async_trait::async_trait]
impl AdapterBindingReply for LinearAdapter {
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
        let api = self.agent_api_for_connection(connection).await?;

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
            return Ok(Some(super::external_id::comment(comment_id)));
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
                Ok(comment_id.as_deref().map(super::external_id::comment))
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
}

#[async_trait::async_trait]
impl AdapterMessageReply for LinearAdapter {
    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        _target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        self.send_reply_for_binding(binding, connection, rule, message)
            .await
    }
}

#[async_trait::async_trait]
impl AdapterDirectDestination for LinearAdapter {
    /// Post a comment on a Linear issue. Destination format:
    /// `issue:<identifier-or-uuid>` (e.g. `issue:SP-123` or
    /// `issue:bf0e…`). The single Linear connection in `adapter_connections`
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

        let api = super::auth::api_for_single_workspace(self).await?;
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
            message_external_id: super::external_id::comment(&comment_id),
            subject_kind: SUBJECT_KIND_ISSUE,
            subject_key: issue_uuid,
            destination_config,
        })
    }
}

#[async_trait::async_trait]
impl AdapterPostBindingFollowup for LinearAdapter {
    /// Link the moatless task back onto the issue an outbound send just bound
    /// to, so the Linear side shows a "source" pill pointing at the task.
    /// Agent-session bindings already carry that link from the session itself.
    async fn on_binding_created(
        &self,
        state: &AppState,
        connection: &AdapterConnection,
        binding: &TaskBinding,
        task_id: &str,
    ) {
        if binding.subject_kind != SUBJECT_KIND_ISSUE {
            return;
        }
        let api = match self.agent_api_for_connection(connection).await {
            Ok(api) => api,
            Err(e) => {
                warn!(
                    connection_id = %connection.id,
                    error = %e,
                    "Failed to resolve Linear app config for binding follow-up"
                );
                return;
            }
        };
        super::issues::attach_parent_task_link(state, &api, &binding.subject_key, task_id).await;
    }
}

#[async_trait::async_trait]
impl AdapterInboundPresentation for LinearAdapter {
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
}

#[async_trait::async_trait]
impl AdapterBindingStatus for LinearAdapter {
    fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
        status::binding_status_key(binding)
    }

    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        status::binding_label_and_url(binding)
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

        let row = sqlx::query_as::<_, (serde_json::Value,)>(
            "SELECT transport_config FROM adapter_connections WHERE id = $1",
        )
        .bind(&binding.connection_id)
        .fetch_optional(&self.db)
        .await;
        let transport_config = match row {
            Ok(Some((transport_config,))) => transport_config,
            Ok(None) => {
                warn!(
                    binding_id = %binding.id,
                    connection_id = %binding.connection_id,
                    "Linear binding status skipped: connection is gone"
                );
                return None;
            }
            Err(e) => {
                warn!(
                    binding_id = %binding.id,
                    connection_id = %binding.connection_id,
                    error = %e,
                    "Linear binding status skipped: connection lookup failed"
                );
                return None;
            }
        };

        let api = match self
            .agent_api_for_transport(org_id, &transport_config)
            .await
        {
            Ok(api) => api,
            Err(e) => {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to resolve Linear app config for status refresh"
                );
                return None;
            }
        };

        match api.get_issue(issue_id).await {
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

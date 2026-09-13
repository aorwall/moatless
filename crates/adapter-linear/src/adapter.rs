//! The Linear adapter's [`Adapter`] trait implementation.
//!
//! The webhook dispatcher lives in `handlers.rs`; this module owns the outbound
//! `agentActivityCreate` calls and the workspace-scoped configuration (webhook
//! secret, OAuth client credentials).

use std::sync::Arc;

use tracing::warn;

use moatless_adapter_model::apps::AdapterAppReader;
use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDirectDestination,
    AdapterInboundPresentation, AdapterMessageReply, AdapterOutboundContent,
    AdapterPostBindingFollowup,
};
use moatless_adapter_model::connections::{ConnectionLookup, ConnectionReader};
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_adapter_model::outbound::OutboundReply;
use moatless_adapter_model::registry::Adapters;
use moatless_ports::identity::AppCredentials;
use moatless_ports::task::TaskReader;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{AdapterKind, ExternalStatus, InboundMessage};
use moatless_types::loops::LoopContext;
use moatless_types::loops::SubscriptionMatch;

use super::SUBJECT_KIND_ISSUE;
use super::agent_api::LinearAgentApi;
use super::auth::LinearCredentials;
use super::status;

/// Linear adapter.
pub struct LinearAdapter {
    /// Global webhook secret from env (`LINEAR_CHANNEL_WEBHOOK_SECRET`). Used
    /// as a fallback when the connection has no `webhook_secret` set.
    pub(crate) global_webhook_secret: Option<String>,
    /// How this Adapter's outbound calls authenticate, and which host they
    /// reach. Owned by [`auth`](super::auth), which is the only module that
    /// reads them.
    credentials: LinearCredentials,
    /// The app-level OAuth row this Adapter authenticates with.
    app_tokens: Arc<dyn AppCredentials>,
    /// Runtime config the webhook route and the binding follow-up read.
    config: Option<Arc<moatless_core::config::Config>>,
    /// The stored Linear apps. Present on the DB-backed constructor; the
    /// test-only ones never reach a stored app.
    apps: Option<Arc<dyn AdapterAppReader>>,
    /// Task store the post-binding follow-up reads to title the attachment
    /// linking back onto the Linear issue.
    ///
    /// `None` in test-only constructors; the composition root injects it
    /// through [`with_tasks`](Self::with_tasks).
    tasks: Option<Arc<dyn TaskReader>>,
    /// Ingress port the webhook route reads through this adapter rather than an
    /// `AppState`.
    ///
    /// Present on the composition-root instance (see
    /// [`with_ingress_ports`](Self::with_ingress_ports)); absent on test
    /// constructors, which never reach [`super::ingress::receive`].
    adapter_connections: Option<Arc<dyn ConnectionLookup>>,
    /// The same rows read on a caller's behalf, for the `/api/v1/linear/*`
    /// endpoints that resolve the connection a request may see.
    connection_reader: Option<Arc<dyn ConnectionReader>>,
    loops: Option<Arc<dyn LoopReader>>,
    adapter_delivery: Option<Arc<dyn DeliveryAdmission>>,
}

/// The internal error for an ingress port `LinearAdapter` never received: a
/// composition-root bug, not a request fault.
fn missing_port(name: &str) -> AdapterError {
    SharedAdapterError::Internal(format!("Linear adapter missing {name} port")).into()
}

/// Build an outbound Linear comment body: the reply text plus the optional
/// `Moatless task:` footer.
///
/// The footer URL doubles as the echo-filter source: when the comment's own
/// webhook echo arrives, the authoring task id is recovered from that link (see
/// `comments::handle_comment_created`).
fn build_comment_body(message: &OutboundReply) -> String {
    let mut body = message.text.clone();
    if let Some(ref footer) = message.footer {
        body.push_str(footer);
    }
    body
}

impl LinearAdapter {
    /// Register `LinearAdapter`'s capabilities into `adapters`. See
    /// [`moatless_loops::adapters::Adapters`]. `pub` so the integration harness
    /// can build a Linear-only registry directly.
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        let kind = AdapterKind::Linear;
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self.clone());
        adapters.register_direct_destination(self.clone());
        adapters.register_post_binding_followup(self.clone());
        adapters.register_inbound_presentation(kind, self.clone());
        adapters.register_binding_status(kind, self);
    }

    /// Build the Adapter from the one place its credentials come from.
    ///
    /// Taking [`LinearConfig`] rather than loose fields keeps the Adapter's
    /// credentials provably the same ones `AppState`-side paths use, and
    /// doubles as the test injection seam via `api_base_url`.
    pub fn new(
        config: &moatless_types::adapter::LinearConfig,
        app_tokens: Arc<dyn AppCredentials>,
    ) -> Self {
        Self {
            global_webhook_secret: config.webhook_secret.clone(),
            credentials: LinearCredentials::from(config),
            app_tokens,
            config: None,
            apps: None,
            tasks: None,
            adapter_connections: None,
            connection_reader: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    pub fn from_db(
        config: Arc<moatless_core::config::Config>,
        app_tokens: Arc<dyn AppCredentials>,
        apps: Arc<dyn AdapterAppReader>,
    ) -> Self {
        Self {
            global_webhook_secret: config.linear.webhook_secret.clone(),
            credentials: LinearCredentials::from(&config.linear),
            app_tokens,
            config: Some(config),
            apps: Some(apps),
            tasks: None,
            adapter_connections: None,
            connection_reader: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    /// Inject the task store the post-binding follow-up needs. The composition
    /// root calls this; test constructors leave it unset because no test drives
    /// the follow-up.
    pub fn with_tasks(mut self, tasks: Arc<dyn TaskReader>) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// Inject the ports [`super::ingress::receive`] reads, so the webhook route
    /// reads them off the adapter instead of an `AppState`.
    ///
    /// `db` and `config` already live on the adapter, so only the
    /// request-scoped ports are carried here; test constructors leave them
    /// unset.
    pub fn with_ingress_ports(
        mut self,
        adapter_connections: Arc<dyn ConnectionLookup>,
        connection_reader: Arc<dyn ConnectionReader>,
        loops: Arc<dyn LoopReader>,
        adapter_delivery: Arc<dyn DeliveryAdmission>,
    ) -> Self {
        self.adapter_connections = Some(adapter_connections);
        self.connection_reader = Some(connection_reader);
        self.loops = Some(loops);
        self.adapter_delivery = Some(adapter_delivery);
        self
    }

    /// The connection store the webhook route hands
    /// [`super::ingress::receive`].
    pub(super) fn require_adapter_connections(
        &self,
    ) -> Result<&dyn ConnectionLookup, AdapterError> {
        self.adapter_connections
            .as_deref()
            .ok_or_else(|| missing_port("adapter_connections"))
    }

    /// The connection store the `/api/v1/linear/*` handlers resolve through.
    pub(super) fn require_connection_reader(&self) -> Result<&dyn ConnectionReader, AdapterError> {
        self.connection_reader
            .as_deref()
            .ok_or_else(|| missing_port("connection_reader"))
    }

    /// The loop store the webhook route hands [`super::ingress::receive`].
    pub(super) fn require_loops(&self) -> Result<&dyn LoopReader, AdapterError> {
        self.loops.as_deref().ok_or_else(|| missing_port("loops"))
    }

    /// The delivery seam the webhook route hands [`super::ingress::receive`].
    pub(super) fn require_adapter_delivery(&self) -> Result<&dyn DeliveryAdmission, AdapterError> {
        self.adapter_delivery
            .as_deref()
            .ok_or_else(|| missing_port("adapter_delivery"))
    }

    /// The runtime config the webhook route hands [`super::ingress::receive`].
    /// Present on the DB-backed constructor ([`from_db`](Self::from_db)); the
    /// composition root builds Linear that way.
    pub(super) fn require_config(&self) -> Result<&moatless_core::config::Config, AdapterError> {
        self.config.as_deref().ok_or_else(|| missing_port("config"))
    }

    /// The stored Linear apps a webhook resolves its client through. Present on
    /// [`from_db`](Self::from_db), which is how the composition root builds it.
    pub(super) fn require_apps(&self) -> Result<&dyn AdapterAppReader, AdapterError> {
        self.apps
            .as_deref()
            .ok_or_else(|| missing_port("adapter apps"))
    }

    /// Build a Linear Agents API client for the given workspace. The app-level
    /// OAuth credential this Adapter's outbound calls use.
    pub(crate) fn app_tokens(&self) -> &Arc<dyn AppCredentials> {
        &self.app_tokens
    }

    pub(crate) fn agent_api_for_org(&self, org_id: &str) -> LinearAgentApi {
        self.credentials
            .api_for_org(self.app_tokens.clone(), org_id)
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
        let Some(apps) = self.apps.as_deref() else {
            return Ok(self.agent_api_for_org(org_id));
        };
        let app_key = moatless_adapter_model::apps::app_key_from_transport_config(transport_config);
        let Some(app) = apps.app("linear", app_key).await? else {
            return Err(SharedAdapterError::AdapterNotConfigured(format!(
                "linear app '{app_key}'"
            ))
            .into());
        };
        Ok(LinearCredentials::from(app).api_for_org(self.app_tokens.clone(), org_id))
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
    /// Linear comments allow generous bodies; cap well below the documented 50k
    /// limit to leave headroom for the `Moatless task:` footer.
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
        Err(SharedAdapterError::BadRequest(
            "Linear outbound now goes through send_reply_for_binding (Agents API)".into(),
        )
        .into())
    }

    /// Reply to a bound Linear destination: `commentCreate` on the session's or
    /// issue's bound Linear issue, keyed by `subject_kind`.
    ///
    /// The terminal `response` activity closing the Linear agent session is
    /// posted separately by `handle_agent_completed`, not here — keeps that
    /// lifecycle owned by a single emission point.
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
                    SharedAdapterError::AdapterApi("Linear commentCreate missing comment.id".into())
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
                SharedAdapterError::BadRequest(
                    "Linear binding missing session_id in destination_config".into(),
                )
            })?;

        let issue_uuid = binding
            .destination_config
            .get("issue_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::BadRequest(
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
    /// `issue:<identifier-or-uuid>` (e.g. `issue:SP-123`).
    ///
    /// Uses the single Linear connection in `adapter_connections` as workspace
    /// context; multi-org callers must use the typed `/api/v1/linear/*`
    /// endpoints instead.
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<moatless_adapter_model::outbound::DestinationPostResult, AdapterError> {
        let id_or_key = destination.strip_prefix("issue:").unwrap_or(destination);
        if id_or_key.is_empty() {
            return Err(SharedAdapterError::BadRequest(
                "Linear destination must be `issue:<SP-N>` or `issue:<uuid>`".into(),
            )
            .into());
        }

        let api = super::auth::api_for_single_workspace(self).await?;
        let issue = api.get_issue(id_or_key).await?;
        let issue_uuid = issue
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::AdapterApi("Linear issue payload missing id".into())
            })?
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
                SharedAdapterError::AdapterApi("Linear commentCreate missing comment.id".into())
            })?
            .to_string();

        let mut destination_config = serde_json::json!({
            "issue_id": issue_uuid,
            "issue_identifier": identifier,
        });
        if let Some(u) = url {
            destination_config["url"] = serde_json::Value::String(u);
        }

        Ok(moatless_adapter_model::outbound::DestinationPostResult {
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
        connection: &AdapterConnection,
        binding: &TaskBinding,
        task_id: &str,
    ) {
        if binding.subject_kind != SUBJECT_KIND_ISSUE {
            return;
        }
        // Owned ports the follow-up needs; both are set by the composition root
        // and only absent in test-only construction, which never gets here.
        let (Some(config), Some(tasks)) = (self.config.as_deref(), self.tasks.as_deref()) else {
            warn!(
                connection_id = %connection.id,
                "Linear binding follow-up skipped: adapter built without config/task store"
            );
            return;
        };
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
        super::task_links::attach_parent_task_link(
            config,
            tasks,
            &api,
            &binding.subject_key,
            task_id,
        )
        .await;
    }
}

/// Reply guidance (posting and mention conventions) appended to a Linear-fired
/// task's firing system prompt.
///
/// Held as a module constant since both the [`AdapterInboundPresentation`] impl
/// below and the issue status-entry firing path in [`issues`](super::issues)
/// reference the same text.
pub(super) const REPLY_INSTRUCTIONS: &str = "## Linear Agent Notes\n\
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
       external URLs — no need to include it in the reply body.";

#[async_trait::async_trait]
impl AdapterInboundPresentation for LinearAdapter {
    async fn reply_instructions(&self) -> Option<String> {
        Some(REPLY_INSTRUCTIONS.to_string())
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

        let Some(connections) = self.adapter_connections.as_ref() else {
            warn!(
                binding_id = %binding.id,
                "Linear binding status skipped: adapter has no connection port"
            );
            return None;
        };
        let row = connections.by_id_trusted(&binding.connection_id).await;
        let transport_config = match row {
            Ok(Some(connection)) => connection.transport_config,
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
    use moatless_adapter_model::footer::parse_task_id_from_body;

    fn reply(text: &str, footer: Option<&str>) -> OutboundReply {
        OutboundReply {
            text: text.to_string(),
            provenance: None,
            footer: footer.map(str::to_string),
            author_user_id: None,
            is_error: false,
            files: vec![],
            presentation: None,
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

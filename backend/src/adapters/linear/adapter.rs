//! Linear adapter — `Adapter` trait impl + webhook parsing.

use std::fmt::Write as _;
use std::time::Duration;

use moka::future::Cache;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, InboundMessage, OutboundReply, TaskBinding,
    TaskTriggerRoute, TriggerContext,
};
use crate::adapters::{Adapter, AdapterKind, DestinationPostResult};
use crate::db::DbPool;
use crate::state::AppState;

use super::api::LinearApi;
use super::config::LinearRuleConfig;

/// Agent comment marker used to filter our own outbound from inbound contexts.
pub(crate) const AGENT_MARKER: &str = "<!-- moatless-agent -->";

/// Result of parsing a Linear webhook for a single listener.
pub enum LinearWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// Issue transitioned into one of the rule's `terminal_states` and the
    /// rule has `close_task_on_close=true` — close associated tasks.
    CloseTask(IssueCloseInfo),
    /// Webhook should be ignored (filter miss, unsupported event, etc.).
    Ignore,
}

/// Metadata about a closed Linear issue, captured at parse time and used
/// downstream to compose the auto-close chat notice.
#[derive(Debug, Clone)]
pub struct IssueCloseInfo {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state_name: String,
    pub actor_name: String,
}

/// Linear adapter.
pub struct LinearAdapter {
    /// Global webhook secret from env (`LINEAR_CHANNEL_WEBHOOK_SECRET`).
    /// Used as a fallback when the connection has no `webhook_secret` set.
    pub(crate) global_webhook_secret: Option<String>,
    /// Global API token from env (`LINEAR_CHANNEL_TOKEN`).
    pub(crate) global_api_token: Option<String>,
    /// Database pool for looking up provider tokens.
    db: DbPool,
    /// Deduplication cache: webhookId → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
}

impl LinearAdapter {
    pub fn new(
        global_webhook_secret: Option<String>,
        global_api_token: Option<String>,
        db: DbPool,
    ) -> Self {
        Self {
            global_webhook_secret,
            global_api_token,
            db,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Resolve a Linear API token for outbound calls. Order:
    /// `provider_tokens(user_id, provider="linear")` → global env.
    pub(crate) async fn resolve_token_for_user(
        &self,
        user_id: &str,
    ) -> Result<String, AdapterError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = 'linear' LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| AdapterError::AdapterApi(format!("Failed to fetch Linear token: {e}")))?;

        if let Some(token) = row {
            return Ok(token);
        }
        if let Some(ref token) = self.global_api_token {
            return Ok(token.clone());
        }
        Err(AdapterError::AdapterApi(format!(
            "No Linear token available: user {user_id} has no provider token and \
             LINEAR_CHANNEL_TOKEN env is not set"
        )))
    }

    /// Resolve a token for reactions: prefer the binding's `filter_user_id`,
    /// then global env. Distinct from `resolve_token_for_user` which always
    /// requires a user id.
    pub(crate) async fn resolve_reaction_token(
        &self,
        user_id: &str,
    ) -> Result<String, AdapterError> {
        if !user_id.is_empty() {
            return self.resolve_token_for_user(user_id).await;
        }
        self.global_api_token.clone().ok_or_else(|| {
            AdapterError::AdapterApi(
                "No Linear token available for reactions: binding has no filter_user_id and \
                 LINEAR_CHANNEL_TOKEN env is not set"
                    .into(),
            )
        })
    }

    /// Mark a `webhookId` as processed; returns `true` if this is the first
    /// time we see it (caller should proceed), or `false` if it's a dup.
    pub(crate) async fn mark_seen(&self, webhook_id: &str) -> bool {
        if self.dedup_cache.get(webhook_id).await.is_some() {
            return false;
        }
        self.dedup_cache.insert(webhook_id.to_string(), ()).await;
        true
    }

    /// Extract the issue UUID from a `TaskBinding`'s `destination_config`.
    pub(crate) fn parse_binding_target(binding: &TaskBinding) -> Result<String, AdapterError> {
        binding
            .destination_config
            .get("issue_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                AdapterError::Internal("Missing issue_id in binding destination_config".into())
            })
    }

    /// Extract the issue UUID from a `BindingContext`'s properties.
    pub(crate) fn parse_thread_props(thread: &BindingContext) -> Result<String, AdapterError> {
        thread
            .destination_config
            .get("issue_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AdapterError::Internal("Missing issue_id in thread properties".into()))
    }
}

/// Compact view of a Linear issue inside a webhook payload.
///
/// Linear's webhook envelope nests the issue in `data` for `Issue` events and
/// alongside the comment in `data.issue` for `Comment` events. This struct
/// parses just the fields we care about and tolerates either shape.
#[derive(Debug, Clone, Deserialize)]
pub struct IssuePayload {
    pub id: String,
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub state: Option<StatePayload>,
    /// Label set may arrive either as `labels: [{name}, ...]` or
    /// `labels: { nodes: [{name}, ...] }`. We accept both via custom deserialize.
    #[serde(default, deserialize_with = "deserialize_labels")]
    pub labels: Vec<String>,
    #[serde(default, rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(default)]
    pub team: Option<TeamPayload>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatePayload {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TeamPayload {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentPayload {
    pub id: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, rename = "issueId")]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub issue: Option<IssuePayload>,
    #[serde(default)]
    pub user: Option<UserPayload>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserPayload {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinearWebhookEnvelope {
    /// `"create"` | `"update"` | `"remove"`.
    pub action: String,
    /// `"Issue"` | `"Comment"` | etc.
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: serde_json::Value,
    #[serde(default, rename = "organizationId")]
    pub organization_id: Option<String>,
    #[serde(default, rename = "webhookId")]
    pub webhook_id: Option<String>,
    /// Captured for state-change diffing.
    #[serde(default, rename = "updatedFrom")]
    pub updated_from: Option<serde_json::Value>,
    #[serde(default)]
    pub actor: Option<UserPayload>,
}

/// Linear sends labels as either an array of objects or a `nodes` wrapper.
fn deserialize_labels<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = serde_json::Value::deserialize(de)?;
    fn extract(v: &serde_json::Value) -> Vec<String> {
        v.as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.get("name").and_then(serde_json::Value::as_str))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }
    if let Some(nodes) = value.get("nodes") {
        Ok(extract(nodes))
    } else if value.is_array() {
        Ok(extract(&value))
    } else if value.is_null() {
        Ok(vec![])
    } else {
        Err(D::Error::custom("unexpected labels shape"))
    }
}

/// Decide what to do with an `Issue` event for a single listener.
pub fn parse_issue_event(
    envelope: &LinearWebhookEnvelope,
    rule: &TaskTriggerRoute,
    rule_config: &LinearRuleConfig,
) -> Result<LinearWebhookResult, AdapterError> {
    let issue: IssuePayload = serde_json::from_value(envelope.data.clone())
        .map_err(|e| AdapterError::BadRequest(format!("Invalid Issue payload: {e}")))?;

    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();

    let state_name = issue.state.as_ref().map_or("", |s| s.name.as_str());

    // Terminal-state close takes precedence over trigger evaluation: a single
    // Issue.update can both leave a trigger state and enter a terminal state.
    if envelope.action == "update"
        && rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_name)
    {
        return Ok(LinearWebhookResult::CloseTask(IssueCloseInfo {
            issue_id: issue.id,
            identifier: issue.identifier,
            title: issue.title,
            url: issue.url.unwrap_or_default(),
            state_name: state_name.to_string(),
            actor_name: envelope
                .actor
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_default(),
        }));
    }

    // Trigger condition: labels superset AND state in trigger_states.
    let labels_ok = rule_config
        .require_labels
        .iter()
        .all(|required| issue.labels.iter().any(|l| l == required));
    let state_ok = rule_config.trigger_states.is_empty()
        || rule_config.trigger_states.iter().any(|s| s == state_name);

    if !(labels_ok && state_ok) {
        debug!(
            issue = %issue.identifier,
            state = %state_name,
            "Linear issue did not pass trigger filter for rule"
        );
        return Ok(LinearWebhookResult::Ignore);
    }

    // Distinguish "issue created already in trigger state" from "issue
    // updated INTO a trigger state". For updates we additionally require the
    // state to have actually changed; otherwise an unrelated update (e.g.
    // assignee, priority) would re-fire the trigger.
    let action_prefix = match envelope.action.as_str() {
        "create" => "opened",
        "update" => {
            let prev_state_id = envelope
                .updated_from
                .as_ref()
                .and_then(|v| v.get("stateId"))
                .and_then(serde_json::Value::as_str);
            let new_state_id = issue.state.as_ref().map(|s| s.id.as_str());
            match (prev_state_id, new_state_id) {
                (Some(prev), Some(now)) if prev != now => "state_changed",
                _ => return Ok(LinearWebhookResult::Ignore),
            }
        }
        _ => return Ok(LinearWebhookResult::Ignore),
    };

    let mut text = format!("## Linear issue {} — {}", issue.identifier, issue.title);
    if !state_name.is_empty() {
        let _ = write!(text, "\n\n**State:** {state_name}");
    }
    if let Some(ref desc) = issue.description {
        if !desc.is_empty() {
            let _ = write!(text, "\n\n{desc}");
        }
    }
    if let Some(ref url) = issue.url {
        if !url.is_empty() {
            let _ = write!(text, "\n\n{url}");
        }
    }

    let user_id = rule.effective_user_id().to_string();

    Ok(LinearWebhookResult::Message(Box::new(InboundMessage {
        provider: AdapterKind::Linear,
        source_id: rule.source_matcher.clone(),
        thread_id: None, // inbound event — no existing thread context
        message_thread_id: issue.id.clone(),
        user_id,
        text,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "issue_id": issue.id,
            "identifier": issue.identifier,
            "team_id": team_id,
            "state_name": state_name,
        }),
        provider_context: serde_json::json!({}),
        external_event_id: Some(format!("linear:issue:{action_prefix}:{}", issue.id)),
    })))
}

/// Decide what to do with a `Comment` event. No per-listener filter — comment
/// delivery follows existing bindings.
pub fn parse_comment_event(
    envelope: &LinearWebhookEnvelope,
) -> Result<LinearWebhookResult, AdapterError> {
    let comment: CommentPayload = serde_json::from_value(envelope.data.clone())
        .map_err(|e| AdapterError::BadRequest(format!("Invalid Comment payload: {e}")))?;

    if comment.body.starts_with(AGENT_MARKER) {
        debug!(comment_id = %comment.id, "Skipping Linear comment from our own agent");
        return Ok(LinearWebhookResult::Ignore);
    }

    let issue_id = comment
        .issue_id
        .clone()
        .or_else(|| comment.issue.as_ref().map(|i| i.id.clone()))
        .ok_or_else(|| AdapterError::BadRequest("Linear comment missing issue id".into()))?;

    let identifier = comment
        .issue
        .as_ref()
        .map(|i| i.identifier.clone())
        .unwrap_or_default();
    let team_id = comment
        .issue
        .as_ref()
        .and_then(|i| {
            i.team_id
                .clone()
                .or_else(|| i.team.as_ref().map(|t| t.id.clone()))
        })
        .unwrap_or_default();

    let user_id = comment
        .user
        .as_ref()
        .map(|u| u.id.clone())
        .or_else(|| envelope.actor.as_ref().map(|a| a.id.clone()))
        .unwrap_or_default();

    Ok(LinearWebhookResult::Message(Box::new(InboundMessage {
        provider: AdapterKind::Linear,
        source_id: team_id.clone(),
        thread_id: Some(issue_id.clone()),
        message_thread_id: comment.id.clone(),
        user_id,
        text: comment.body,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "issue_id": issue_id,
            "identifier": identifier,
            "team_id": team_id,
            "comment_id": comment.id,
        }),
        provider_context: serde_json::json!({}),
        external_event_id: Some(format!("linear:comment:{}", comment.id)),
    })))
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
    /// 50k limit to leave headroom for the agent marker and footer.
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
        thread: &BindingContext,
        config: &TriggerContext,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let issue_id = Self::parse_thread_props(thread)?;
        let token = self
            .resolve_token_for_user(config.effective_user_id())
            .await?;
        let api = LinearApi::new(&token);

        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }
        let comment_id = api.comment_create(&issue_id, &body).await?;
        Ok(Some(format!("linear:comment:{comment_id}")))
    }

    /// Post a comment on a Linear issue identified by the destination string.
    ///
    /// Accepted destination formats:
    /// - `issue:<id-or-key>` — e.g. `issue:LIN-123` or the Linear issue UUID
    /// - raw `<id-or-key>` — bare value for backward compatibility
    ///
    /// Authentication uses the global Linear token (`LINEAR_CHANNEL_TOKEN`).
    /// For per-user attribution, use the binding-based `send_reply` path.
    async fn post_to_destination(
        &self,
        destination: &str,
        message: &OutboundReply,
    ) -> Result<DestinationPostResult, AdapterError> {
        let issue_id = parse_linear_destination(destination)?;
        let token = self.global_api_token.clone().ok_or_else(|| {
            AdapterError::AdapterApi(
                "No global Linear token configured (LINEAR_CHANNEL_TOKEN). \
                 Set the env var or use the binding-based reply path."
                    .into(),
            )
        })?;
        let api = LinearApi::new(&token);

        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }
        let comment_id = api.comment_create(&issue_id, &body).await?;

        Ok(DestinationPostResult {
            message_external_id: format!("linear:comment:{comment_id}"),
            subject_kind: "linear_comment",
            subject_key: format!("linear:issue:{issue_id}"),
            destination_config: serde_json::json!({
                "issue_id": issue_id,
                "comment_id": comment_id,
            }),
        })
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&TaskTriggerRoute>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let issue_id = Self::parse_binding_target(binding)?;
        let user_id = match rule {
            Some(r) => r.effective_user_id().to_string(),
            None => binding
                .metadata
                .get("filter_user_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        if user_id.is_empty() {
            return Err(AdapterError::AdapterApi(
                "Cannot resolve Linear token: no task trigger attached to binding".into(),
            ));
        }
        let token = self.resolve_token_for_user(&user_id).await?;
        let api = LinearApi::new(&token);

        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }
        let comment_id = api.comment_create(&issue_id, &body).await?;
        Ok(Some(format!("linear:comment:{comment_id}")))
    }

    async fn fetch_context(
        &self,
        thread: &BindingContext,
        config: &TriggerContext,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
        let issue_id = Self::parse_thread_props(thread)?;
        let user_id = config.effective_user_id();
        let token = match self.resolve_token_for_user(user_id).await {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let api = LinearApi::new(&token);

        let comments = api.list_comments(&issue_id, limit).await?;
        if comments.is_empty() {
            return Ok(None);
        }

        let context = comments
            .iter()
            .filter(|c| !c.body.contains(AGENT_MARKER))
            .filter(|c| {
                let prefixed = format!("linear:comment:{}", c.id);
                !exclude_external_ids.contains(&prefixed)
            })
            .map(|c| format!("@{}: {}", c.author(), c.body))
            .collect::<Vec<_>>()
            .join("\n\n");

        if context.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!("## Issue Comments\n\n{context}")))
        }
    }

    fn reply_instructions(&self) -> Option<String> {
        Some(
            "## Linear Reply Notes\n\
             - Replies are posted as new comments on the originating issue.\n\
             - Plain text or Markdown is supported (no Slack-style @mention syntax).\n\
             - State transitions and label edits are NOT done via reply — use the \
             Linear MCP/CLI when needed."
                .to_string(),
        )
    }

    fn thread_reply_hint(&self, _msg: &InboundMessage) -> Option<String> {
        Some("[New comment on Linear issue]".to_string())
    }

    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let comment_id = parse_linear_comment_id(target_external_id)?;
        let emoji = normalize_linear_emoji(emoji)?;
        let user_id = binding
            .destination_config
            .get("filter_user_id")
            .or_else(|| binding.metadata.get("filter_user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let token = self.resolve_reaction_token(user_id).await?;
        let api = LinearApi::new(&token);
        let id = api.comment_reaction_create(comment_id, emoji).await?;
        Ok(Some(id))
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let comment_id = parse_linear_comment_id(target_external_id)?;
        let emoji = normalize_linear_emoji(emoji)?;
        let user_id = binding
            .destination_config
            .get("filter_user_id")
            .or_else(|| binding.metadata.get("filter_user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let token = self.resolve_reaction_token(user_id).await?;
        let api = LinearApi::new(&token);
        let viewer = api.viewer().await?;
        let reactions = api.list_comment_reactions(comment_id).await?;
        let target = reactions
            .into_iter()
            .find(|r| {
                r.emoji == emoji
                    && r.user.as_ref().is_some_and(|u| u.id == viewer.id)
            })
            .ok_or_else(|| {
                AdapterError::NotFound(format!(
                    "Linear: no `{emoji}` reaction by viewer `{}` on comment {comment_id}",
                    viewer.id
                ))
            })?;
        api.reaction_delete(&target.id).await
    }
}

/// Extract a Linear comment UUID from a `linear:comment:<uuid>` external id.
fn parse_linear_comment_id(external_id: &str) -> Result<&str, AdapterError> {
    external_id
        .strip_prefix("linear:comment:")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AdapterError::BadRequest(format!(
                "Linear reaction target `{external_id}` is not a linear:comment:<id> id"
            ))
        })
}

/// Strip leading/trailing `:` and fail on empty input. Linear accepts emoji
/// shortcodes (e.g. `thumbsup`) directly.
fn normalize_linear_emoji(emoji: &str) -> Result<&str, AdapterError> {
    let trimmed = emoji.trim().trim_matches(':');
    if trimmed.is_empty() {
        return Err(AdapterError::BadRequest("Empty emoji name".into()));
    }
    Ok(trimmed)
}

/// Parse a Linear destination string into the issue identifier.
///
/// Accepted shapes:
/// - `issue:<id-or-key>` — preferred, e.g. `issue:LIN-123` or `issue:<uuid>`
/// - `<id-or-key>` — bare value (backward-compatible fallback)
fn parse_linear_destination(destination: &str) -> Result<String, AdapterError> {
    let raw = destination.trim();
    if raw.is_empty() {
        return Err(AdapterError::BadRequest(
            "Linear destination must not be empty".into(),
        ));
    }
    if let Some(rest) = raw.strip_prefix("issue:") {
        if rest.is_empty() {
            return Err(AdapterError::BadRequest(
                "Linear issue destination must not be empty".into(),
            ));
        }
        return Ok(rest.to_string());
    }
    Ok(raw.to_string())
}

/// Process a Linear webhook: verify signature, parse, fan out to every rule
/// matching the team. Always returns `Ok(())` so the HTTP handler can ack
/// quickly even on benign rejections.
pub async fn resolve_and_process_linear_webhook(
    state: &AppState,
    provider: &LinearAdapter,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> Result<(), AdapterError> {
    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if signature.is_empty() {
        debug!("Linear webhook missing Linear-Signature header — ignoring");
        return Ok(());
    }

    let envelope: LinearWebhookEnvelope = match serde_json::from_slice(body) {
        Ok(env) => env,
        Err(e) => {
            return Err(AdapterError::BadRequest(format!(
                "Invalid Linear webhook payload: {e}"
            )));
        }
    };

    let Some(org_id) = envelope.organization_id.as_deref() else {
        debug!("Linear webhook missing organizationId — ignoring");
        return Ok(());
    };

    let connection = crate::adapters::dao::PgAdapterConnectionRepository(state.db.clone())
        .find_by_scope_value("linear", org_id)
        .await?;
    let Some(connection) = connection else {
        info!(
            org_id,
            "Linear webhook ignored: no provider_connection registered"
        );
        return Ok(());
    };

    let secret = connection
        .webhook_secret
        .as_deref()
        .or(provider.global_webhook_secret.as_deref());
    if let Some(secret) = secret {
        super::signature::verify_signature(secret, body, signature)?;
    } else {
        warn!(
            org_id,
            "Linear webhook arrived but no webhook secret configured on connection or env — refusing"
        );
        return Err(AdapterError::SignatureInvalid);
    }

    if let Some(ref webhook_id) = envelope.webhook_id {
        if !provider.mark_seen(webhook_id).await {
            debug!(webhook_id, "Linear webhook deduplicated");
            return Ok(());
        }
    }

    // Resolve the team_id for routing. Issue events nest team in `data.team`
    // or `data.teamId`; Comment events nest it under `data.issue.team`.
    let team_id = match envelope.event_type.as_str() {
        "Issue" => envelope
            .data
            .get("teamId")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                envelope
                    .data
                    .pointer("/team/id")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_string),
        "Comment" => envelope
            .data
            .pointer("/issue/teamId")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                envelope
                    .data
                    .pointer("/issue/team/id")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_string),
        _ => None,
    };
    let Some(team_id) = team_id else {
        debug!(event_type = %envelope.event_type, "Linear webhook missing team_id — ignoring");
        return Ok(());
    };

    let rules = super::runtime::resolve_provider_event(state, &connection, &team_id).await?;
    if rules.is_empty() {
        info!(team_id = %team_id, "Linear webhook ignored: no task trigger for team");
        return Ok(());
    }

    match envelope.event_type.as_str() {
        "Issue" => {
            for rule in rules {
                if !rule.active {
                    continue;
                }
                let rule_config = LinearRuleConfig::from_rule(&rule);
                let result = parse_issue_event(&envelope, &rule, &rule_config)?;
                spawn_dispatch(state.clone(), connection.clone(), rule, result);
            }
        }
        "Comment" => {
            // Parse once — comment routing is shared across every listener that
            // already has a binding on the issue.
            let result = parse_comment_event(&envelope)?;
            // Pick any active rule under this team to carry along for outbound
            // auth + reply formatting; the runtime fan-out resolves each
            // delivered binding's own rule when sending.
            if let Some(rule) = rules.into_iter().find(|r| r.active) {
                spawn_dispatch(state.clone(), connection.clone(), rule, result);
            }
        }
        other => {
            debug!(event_type = other, "Unsupported Linear event type");
        }
    }

    Ok(())
}

fn spawn_dispatch(
    state: AppState,
    connection: AdapterConnection,
    rule: TaskTriggerRoute,
    result: LinearWebhookResult,
) {
    match result {
        LinearWebhookResult::Message(msg) => {
            tokio::spawn(async move {
                if let Err(e) =
                    super::runtime::route_inbound_message(&state, &connection, &rule, *msg).await
                {
                    warn!(error = %e, "Failed to route Linear inbound message");
                }
            });
        }
        LinearWebhookResult::CloseTask(info) => {
            tokio::spawn(async move {
                if let Err(e) =
                    super::runtime::close_tasks_for_issue(&state, &connection, &rule, &info).await
                {
                    warn!(error = %e, "Failed to close Linear-issue tasks");
                }
            });
        }
        LinearWebhookResult::Ignore => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::models::RoutingMode;
    use crate::scope::Scope;
    use chrono::Utc;

    fn rule_with(
        provider_config: serde_json::Value,
        close_task_on_close: bool,
    ) -> TaskTriggerRoute {
        TaskTriggerRoute {
            id: "ir_test".into(),
            provider: "linear".into(),
            connection_id: "pc_test".into(),
            source_matcher: "team-uuid".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            allow_direct_send: false,
            provider_config: Some(provider_config),
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success: false,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
            team_id: None,
            owner_user_id: Some("alice".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn issue_envelope(
        action: &str,
        state_name: &str,
        labels: &[&str],
        prev_state_id: Option<&str>,
    ) -> LinearWebhookEnvelope {
        let labels_json: Vec<_> = labels
            .iter()
            .map(|n| serde_json::json!({"name": n}))
            .collect();
        LinearWebhookEnvelope {
            action: action.into(),
            event_type: "Issue".into(),
            data: serde_json::json!({
                "id": "issue-uuid-1",
                "identifier": "ENG-42",
                "title": "Reproducible bug",
                "description": "Steps to reproduce",
                "url": "https://linear.app/acme/issue/ENG-42",
                "state": {"id": "state-todo", "name": state_name},
                "labels": labels_json,
                "teamId": "team-uuid",
            }),
            organization_id: Some("org-uuid".into()),
            webhook_id: Some("wh-1".into()),
            updated_from: prev_state_id.map(|prev| serde_json::json!({"stateId": prev})),
            actor: Some(UserPayload {
                id: "linear-actor".into(),
                name: "Linear User".into(),
                email: None,
            }),
        }
    }

    #[test]
    fn create_event_matching_filter_returns_message() {
        let rule = rule_with(
            serde_json::json!({"require_labels":["agent"], "trigger_states":["Todo"]}),
            true,
        );
        let cfg = LinearRuleConfig::from_rule(&rule);
        let env = issue_envelope("create", "Todo", &["agent"], None);
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        match result {
            LinearWebhookResult::Message(msg) => {
                assert_eq!(
                    msg.external_event_id.as_deref(),
                    Some("linear:issue:opened:issue-uuid-1")
                );
                assert_eq!(
                    msg.thread_properties
                        .get("issue_id")
                        .and_then(|v| v.as_str()),
                    Some("issue-uuid-1")
                );
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn create_event_missing_label_is_ignored() {
        let rule = rule_with(
            serde_json::json!({"require_labels":["agent"], "trigger_states":["Todo"]}),
            true,
        );
        let cfg = LinearRuleConfig::from_rule(&rule);
        let env = issue_envelope("create", "Todo", &["bug"], None);
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        assert!(matches!(result, LinearWebhookResult::Ignore));
    }

    #[test]
    fn create_event_wrong_state_is_ignored() {
        let rule = rule_with(
            serde_json::json!({"require_labels":[], "trigger_states":["Todo"]}),
            true,
        );
        let cfg = LinearRuleConfig::from_rule(&rule);
        let env = issue_envelope("create", "In Review", &[], None);
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        assert!(matches!(result, LinearWebhookResult::Ignore));
    }

    #[test]
    fn update_event_into_trigger_state_returns_state_changed() {
        let rule = rule_with(serde_json::json!({"trigger_states":["In Progress"]}), true);
        let cfg = LinearRuleConfig::from_rule(&rule);
        let env = issue_envelope("update", "In Progress", &[], Some("state-old"));
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        match result {
            LinearWebhookResult::Message(msg) => {
                assert_eq!(
                    msg.external_event_id.as_deref(),
                    Some("linear:issue:state_changed:issue-uuid-1")
                );
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn update_event_without_state_change_is_ignored() {
        let rule = rule_with(serde_json::json!({"trigger_states":["Todo"]}), true);
        let cfg = LinearRuleConfig::from_rule(&rule);
        // Same state id before and after — represents an unrelated update
        // (assignee, priority, etc.) and must NOT re-fire the trigger.
        let env = issue_envelope("update", "Todo", &[], Some("state-todo"));
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        assert!(matches!(result, LinearWebhookResult::Ignore));
    }

    #[test]
    fn update_into_terminal_state_returns_close() {
        let rule = rule_with(
            serde_json::json!({"trigger_states":["Todo"], "terminal_states":["Done"]}),
            true,
        );
        let cfg = LinearRuleConfig::from_rule(&rule);
        let env = issue_envelope("update", "Done", &[], Some("state-old"));
        let result = parse_issue_event(&env, &rule, &cfg).expect("ok");
        match result {
            LinearWebhookResult::CloseTask(info) => {
                assert_eq!(info.issue_id, "issue-uuid-1");
                assert_eq!(info.state_name, "Done");
            }
            _ => panic!("expected CloseTask"),
        }
    }

    #[test]
    fn comment_event_with_agent_marker_is_ignored() {
        let env = LinearWebhookEnvelope {
            action: "create".into(),
            event_type: "Comment".into(),
            data: serde_json::json!({
                "id": "c-1",
                "body": format!("{AGENT_MARKER}\nhello"),
                "issueId": "issue-uuid-1",
                "issue": {"id": "issue-uuid-1", "identifier": "ENG-42", "teamId": "team-uuid"},
                "user": {"id": "linear-actor", "name": "Linear User"},
            }),
            organization_id: Some("org-uuid".into()),
            webhook_id: Some("wh-2".into()),
            updated_from: None,
            actor: None,
        };
        let result = parse_comment_event(&env).expect("ok");
        assert!(matches!(result, LinearWebhookResult::Ignore));
    }

    #[test]
    fn comment_event_returns_message() {
        let env = LinearWebhookEnvelope {
            action: "create".into(),
            event_type: "Comment".into(),
            data: serde_json::json!({
                "id": "c-2",
                "body": "Hello agent",
                "issueId": "issue-uuid-1",
                "issue": {"id": "issue-uuid-1", "identifier": "ENG-42", "teamId": "team-uuid"},
                "user": {"id": "linear-actor", "name": "Linear User"},
            }),
            organization_id: Some("org-uuid".into()),
            webhook_id: Some("wh-3".into()),
            updated_from: None,
            actor: None,
        };
        match parse_comment_event(&env).expect("ok") {
            LinearWebhookResult::Message(msg) => {
                assert_eq!(msg.text, "Hello agent");
                assert_eq!(msg.external_event_id.as_deref(), Some("linear:comment:c-2"));
                assert_eq!(msg.thread_id.as_deref(), Some("issue-uuid-1"));
            }
            _ => panic!("expected Message"),
        }
    }

    /// Two listeners with different trigger states evaluating the same payload
    /// must produce different decisions — the core requirement of "different
    /// listeners can configure their own trigger state".
    #[test]
    fn two_listeners_evaluate_independently() {
        let rule_a = rule_with(
            serde_json::json!({"require_labels":["agent"], "trigger_states":["Todo"]}),
            true,
        );
        let rule_b = rule_with(
            serde_json::json!({"require_labels":["bug"], "trigger_states":["In Progress"]}),
            true,
        );
        let cfg_a = LinearRuleConfig::from_rule(&rule_a);
        let cfg_b = LinearRuleConfig::from_rule(&rule_b);

        // Issue.create with state=Todo, labels=[agent]: only rule A fires.
        let env_a = issue_envelope("create", "Todo", &["agent"], None);
        assert!(matches!(
            parse_issue_event(&env_a, &rule_a, &cfg_a).expect("ok"),
            LinearWebhookResult::Message(_)
        ));
        assert!(matches!(
            parse_issue_event(&env_a, &rule_b, &cfg_b).expect("ok"),
            LinearWebhookResult::Ignore
        ));

        // Issue.update transitioning to In Progress with labels=[bug]: only rule B fires.
        let env_b = issue_envelope("update", "In Progress", &["bug"], Some("state-old"));
        assert!(matches!(
            parse_issue_event(&env_b, &rule_a, &cfg_a).expect("ok"),
            LinearWebhookResult::Ignore
        ));
        assert!(matches!(
            parse_issue_event(&env_b, &rule_b, &cfg_b).expect("ok"),
            LinearWebhookResult::Message(_)
        ));
    }
}

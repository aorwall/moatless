//! GitHub PR channel provider — ChannelProvider trait impl + webhook parsing.

use std::fmt::Write;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::inbound;
use crate::channel::models::{
    ChannelConfig, ChannelThread, InboundMessage, OutboundReply,
};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;
use crate::state::AppState;

use super::api::GithubPrApi;
use super::config::GithubPrProviderConfig;

/// Agent comment marker to filter self-replies.
const AGENT_MARKER: &str = "<!-- moatless-agent -->";

/// Result of parsing a GitHub webhook.
pub enum GithubPrWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// Webhook should be ignored (unsupported event, bot self-reply, etc.).
    Ignore,
}

/// Context passed through event handlers to avoid too many function arguments.
struct EventContext<'a> {
    config: &'a ChannelConfig,
    owner: &'a str,
    repo: &'a str,
    sender_login: &'a str,
    db: &'a DbPool,
}

/// GitHub PR channel provider.
pub struct GithubPrProvider {
    /// Global webhook secret from env (`GITHUB_CHANNEL_WEBHOOK_SECRET`).
    global_webhook_secret: Option<String>,
    /// Global bot token from env (`GITHUB_CHANNEL_TOKEN`).
    global_bot_token: Option<String>,
    /// Deduplication cache: delivery_id → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
}

impl GithubPrProvider {
    pub fn new(global_webhook_secret: Option<String>, global_bot_token: Option<String>) -> Self {
        Self {
            global_webhook_secret,
            global_bot_token,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Resolve a GitHub token for API calls.
    ///
    /// Order: per-config bot_token → global env → config owner's provider_token.
    async fn resolve_token(
        &self,
        config: &ChannelConfig,
        db: &DbPool,
    ) -> Result<String, ChannelError> {
        let provider_config = GithubPrProviderConfig::from_config(config);

        // 1. Per-config bot_token
        if let Some(ref token) = provider_config.bot_token {
            return Ok(token.clone());
        }

        // 2. Global env token
        if let Some(ref token) = self.global_bot_token {
            return Ok(token.clone());
        }

        // 3. Config owner's provider_token for "github"
        let row = sqlx::query_scalar::<_, String>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = 'github' LIMIT 1",
        )
        .bind(&config.created_by)
        .fetch_optional(db)
        .await
        .map_err(|e| ChannelError::ProviderApi(format!("Failed to fetch provider token: {e}")))?;

        row.ok_or_else(|| {
            ChannelError::ProviderApi(
                "No GitHub token available: set bot_token in provider_config, GITHUB_CHANNEL_TOKEN env, or connect a GitHub account".into(),
            )
        })
    }

    /// Resolve token for outbound calls where DB pool is not available via trait.
    /// Falls back to global token or per-config token only.
    fn resolve_token_outbound(&self, config: &ChannelConfig) -> Result<String, ChannelError> {
        let provider_config = GithubPrProviderConfig::from_config(config);

        provider_config
            .bot_token
            .or_else(|| self.global_bot_token.clone())
            .ok_or_else(|| {
                ChannelError::ProviderApi(
                    "No GitHub token available for outbound (set bot_token in provider_config or GITHUB_CHANNEL_TOKEN env)".into(),
                )
            })
    }

    /// Extract owner, repo, and PR number from thread properties.
    fn parse_thread_props(thread: &ChannelThread) -> Result<(String, String, u64), ChannelError> {
        let owner = thread
            .properties
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChannelError::Internal("Missing owner in thread properties".into()))?
            .to_string();
        let repo = thread
            .properties
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChannelError::Internal("Missing repo in thread properties".into()))?
            .to_string();
        let pr_number = thread
            .properties
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ChannelError::Internal("Missing pr_number in thread properties".into())
            })?;
        Ok((owner, repo, pr_number))
    }

    /// Parse a GitHub webhook into an `InboundMessage`.
    pub async fn parse_webhook(
        &self,
        delivery_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
        config: &ChannelConfig,
        db: &DbPool,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let provider_config = GithubPrProviderConfig::from_config(config);

        // Dedup by delivery ID
        if self.dedup_cache.get(delivery_id).await.is_some() {
            debug!(delivery_id, "GitHub webhook deduplicated");
            return Ok(GithubPrWebhookResult::Ignore);
        }
        self.dedup_cache.insert(delivery_id.to_string(), ()).await;

        // Filter by events_filter config
        if !provider_config.events_filter.is_empty()
            && !provider_config.events_filter.iter().any(|e| e == event_type)
        {
            debug!(event_type, "GitHub event filtered out by config");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Extract sender login for bot self-reply detection
        let sender_login = payload
            .pointer("/sender/login")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Filter bot self-replies
        if let Some(ref bot_username) = provider_config.bot_username {
            if sender_login == bot_username {
                debug!(sender_login, "Ignoring bot's own event");
                return Ok(GithubPrWebhookResult::Ignore);
            }
        }

        // Extract repo full_name → owner/repo
        let repo_full_name = payload
            .pointer("/repository/full_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let (owner, repo) = repo_full_name
            .split_once('/')
            .ok_or_else(|| ChannelError::BadRequest("Missing repository.full_name".into()))?;

        let ctx = EventContext {
            config,
            owner,
            repo,
            sender_login,
            db,
        };

        match event_type {
            "pull_request" => {
                self.handle_pull_request_event(payload, &provider_config, &ctx)
                    .await
            }
            "issue_comment" => self.handle_issue_comment_event(payload, &ctx).await,
            "pull_request_review_comment" => {
                self.handle_review_comment_event(payload, &ctx).await
            }
            "pull_request_review" => self.handle_review_event(payload, &ctx).await,
            _ => {
                debug!(event_type, "Unsupported GitHub event type");
                Ok(GithubPrWebhookResult::Ignore)
            }
        }
    }

    async fn handle_pull_request_event(
        &self,
        payload: &serde_json::Value,
        provider_config: &GithubPrProviderConfig,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "opened" && action != "reopened" {
            debug!(action, "Ignoring pull_request action");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr = payload.get("pull_request").ok_or_else(|| {
            ChannelError::BadRequest("Missing pull_request in payload".into())
        })?;

        let pr_number = pr
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;
        let title = pr.get("title").and_then(serde_json::Value::as_str).unwrap_or("");
        let body = pr.get("body").and_then(serde_json::Value::as_str).unwrap_or("");
        let head_ref = pr
            .pointer("/head/ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let base_ref = pr
            .pointer("/base/ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Build rich message text
        let mut text = format!(
            "## New Pull Request: #{pr_number}\n\n\
             **Title:** {title}\n\
             **Author:** @{}\n\
             **Branch:** {head_ref} → {base_ref}\n",
            ctx.sender_login,
        );

        if !body.is_empty() {
            let _ = write!(text, "\n### Description\n{body}\n");
        }

        // Optionally include diff
        if provider_config.include_diff {
            match self.resolve_token(ctx.config, ctx.db).await {
                Ok(token) => {
                    let api = GithubPrApi::new(&token);
                    match api
                        .get_pr_diff(ctx.owner, ctx.repo, pr_number, provider_config.max_diff_size)
                        .await
                    {
                        Ok(diff) => {
                            let _ = write!(text, "\n### Diff\n```diff\n{diff}\n```\n");
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to fetch PR diff, continuing without it");
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "No token available for PR diff, continuing without it");
                }
            }
        }

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.config, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.config.source_id.clone(),
            thread_id: None, // New thread
            message_thread_id: pr_number.to_string(),
            user_id,
            text,
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": ctx.owner,
                "repo": ctx.repo,
                "pr_number": pr_number,
            }),
            provider_context: serde_json::json!({}),
            channel_external_id: None,
        })))
    }

    async fn handle_issue_comment_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "created" {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Only process comments on PRs (issue_comment fires for both issues and PRs)
        if payload.pointer("/issue/pull_request").is_none() {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr_number = payload
            .pointer("/issue/number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing issue number".into()))?;

        let comment = payload.get("comment").ok_or_else(|| {
            ChannelError::BadRequest("Missing comment in payload".into())
        })?;

        let comment_body = comment
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let comment_id = comment
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        // Filter comments with agent marker
        if comment_body.contains(AGENT_MARKER) {
            debug!("Ignoring comment with agent marker");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.config, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.config.source_id.clone(),
            thread_id: Some(pr_number.to_string()),
            message_thread_id: pr_number.to_string(),
            user_id,
            text: comment_body.to_string(),
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": ctx.owner,
                "repo": ctx.repo,
                "pr_number": pr_number,
                "comment_id": comment_id,
            }),
            provider_context: serde_json::json!({}),
            channel_external_id: Some(comment_id.to_string()),
        })))
    }

    async fn handle_review_comment_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "created" {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr_number = payload
            .pointer("/pull_request/number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;

        let comment = payload.get("comment").ok_or_else(|| {
            ChannelError::BadRequest("Missing comment in payload".into())
        })?;

        let comment_body = comment
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let comment_id = comment
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let diff_hunk = comment
            .get("diff_hunk")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let path = comment
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Filter comments with agent marker
        if comment_body.contains(AGENT_MARKER) {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Include diff context in the message
        let text = if diff_hunk.is_empty() {
            comment_body.to_string()
        } else {
            format!(
                "**Review comment on `{path}`:**\n\n```diff\n{diff_hunk}\n```\n\n{comment_body}"
            )
        };

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.config, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.config.source_id.clone(),
            thread_id: Some(pr_number.to_string()),
            message_thread_id: pr_number.to_string(),
            user_id,
            text,
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": ctx.owner,
                "repo": ctx.repo,
                "pr_number": pr_number,
                "comment_id": comment_id,
            }),
            provider_context: serde_json::json!({}),
            channel_external_id: Some(comment_id.to_string()),
        })))
    }

    async fn handle_review_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "submitted" {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr_number = payload
            .pointer("/pull_request/number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;

        let review = payload.get("review").ok_or_else(|| {
            ChannelError::BadRequest("Missing review in payload".into())
        })?;

        let review_body = review
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let review_state = review
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Ignore reviews with empty body
        if review_body.is_empty() {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Filter comments with agent marker
        if review_body.contains(AGENT_MARKER) {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let state_label = match review_state {
            "approved" => "Approved",
            "changes_requested" => "Changes Requested",
            "commented" => "Review Comment",
            _ => "Review",
        };

        let text = format!("**{state_label}:**\n\n{review_body}");

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.config, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.config.source_id.clone(),
            thread_id: Some(pr_number.to_string()),
            message_thread_id: pr_number.to_string(),
            user_id,
            text,
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": ctx.owner,
                "repo": ctx.repo,
                "pr_number": pr_number,
            }),
            provider_context: serde_json::json!({}),
            channel_external_id: None,
        })))
    }

    /// Resolve a GitHub login to a system user, falling back to config owner.
    async fn resolve_user_or_fallback(
        &self,
        github_login: &str,
        config: &ChannelConfig,
        db: &DbPool,
    ) -> String {
        if let Ok(Some(user_id)) = self.resolve_user(github_login, db).await {
            user_id
        } else {
            debug!(
                github_login,
                fallback = %config.effective_user_id(),
                "GitHub user not mapped, using config owner"
            );
            config.effective_user_id().to_string()
        }
    }
}

#[async_trait::async_trait]
impl ChannelProvider for GithubPrProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::GithubPr
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        // Look up by provider_login (GitHub username)
        let user_id = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM provider_tokens WHERE provider = 'github' AND provider_login = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(ChannelError::Db)?;

        Ok(user_id)
    }

    async fn send_reply(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let (owner, repo, pr_number) = Self::parse_thread_props(thread)?;
        let token = self.resolve_token_outbound(config)?;
        let api = GithubPrApi::new(&token);

        // Add agent marker and optional task URL
        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }

        api.post_comment(&owner, &repo, pr_number, &body).await?;

        Ok(None)
    }

    async fn fetch_context(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, ChannelError> {
        let (owner, repo, pr_number) = Self::parse_thread_props(thread)?;

        let token = match self.resolve_token_outbound(config) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let api = GithubPrApi::new(&token);

        let comments = api.list_comments(&owner, &repo, pr_number, limit).await?;

        if comments.is_empty() {
            return Ok(None);
        }

        let context = comments
            .iter()
            .filter(|c| {
                // Exclude agent's own comments from context
                c.body
                    .as_deref()
                    .is_none_or(|b| !b.contains(AGENT_MARKER))
            })
            .filter(|c| {
                // Exclude comments already delivered to the task
                !exclude_external_ids.contains(&c.id.to_string())
            })
            .map(|c| {
                format!(
                    "@{}: {}",
                    c.author(),
                    c.body.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        if context.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!("## PR Comments\n\n{context}")))
        }
    }
}

/// Process a GitHub webhook: verify signature, parse, and route.
///
/// Called from the HTTP handler.
pub async fn resolve_and_process_github_webhook(
    state: &AppState,
    github_provider: &GithubPrProvider,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> Result<(), ChannelError> {
    // Extract delivery ID for dedup
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Extract event type
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event_type.is_empty() {
        return Err(ChannelError::BadRequest(
            "Missing X-GitHub-Event header".into(),
        ));
    }

    // Quick-parse to get repo full_name for config lookup
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| ChannelError::BadRequest(format!("Invalid JSON: {e}")))?;

    let repo_full_name = payload
        .pointer("/repository/full_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChannelError::BadRequest("Missing repository.full_name".into()))?;

    info!(event_type, repo = repo_full_name, delivery_id, "GitHub PR webhook received");

    // Look up config
    let config = state
        .channel_configs
        .find_by_provider_source("github_pr", repo_full_name)
        .await?;

    // Extract sender login early for auto-creation `created_by` field
    let sender_login = payload
        .pointer("/sender/login")
        .and_then(serde_json::Value::as_str);

    let config = match config {
        Some(c) if c.active => c,
        Some(_) => {
            info!(repo = repo_full_name, "GitHub PR webhook ignored: config inactive");
            return Ok(());
        }
        None => {
            // Try auto-create from wildcard template (only if auto_create is enabled)
            let wildcard = state
                .channel_configs
                .find_wildcard("github_pr", None)
                .await
                .map_err(ChannelError::Db)?;

            let Some(template) = wildcard else {
                info!(repo = repo_full_name, "GitHub PR webhook ignored: no config and no wildcard");
                return Ok(());
            };

            let template_config = GithubPrProviderConfig::from_config(&template);
            if !template_config.auto_create {
                info!(repo = repo_full_name, "GitHub PR webhook ignored: wildcard exists but auto_create is disabled");
                return Ok(());
            }

            match super::super::defaults::create_config_from_wildcard(
                &*state.channel_configs,
                "github_pr",
                repo_full_name,
                Some(repo_full_name.to_string()),
                template.team_id.as_deref(),
                sender_login.unwrap_or("system"),
            )
            .await
            {
                Ok(Some(config)) => {
                    info!(config_id = %config.id, repo = repo_full_name, "Auto-created GitHub PR config from wildcard");
                    config
                }
                Ok(None) => {
                    info!(repo = repo_full_name, "GitHub PR webhook ignored: wildcard clone returned None");
                    return Ok(());
                }
                Err(e) => {
                    warn!(repo = repo_full_name, error = %e, "Failed to auto-create GitHub PR config from wildcard");
                    return Ok(());
                }
            }
        }
    };

    // Verify webhook signature
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let provider_config = GithubPrProviderConfig::from_config(&config);

    // Resolve secret: per-config → global → skip with warning
    let secret = provider_config
        .webhook_secret
        .as_deref()
        .or(github_provider.global_webhook_secret.as_deref());

    if let Some(secret) = secret {
        super::signature::verify_signature(secret, body, signature)?;
    } else if !signature.is_empty() {
        warn!(repo = repo_full_name, "GitHub webhook has signature but no secret configured, skipping verification");
    }

    // Parse the webhook event
    let result = github_provider
        .parse_webhook(delivery_id, event_type, &payload, &config, &state.db)
        .await?;

    match result {
        GithubPrWebhookResult::Message(msg) => {
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    inbound::process_inbound(&state_clone, *msg).await
                {
                    warn!(error = %e, "Failed to process GitHub PR inbound message");
                }
            });
        }
        GithubPrWebhookResult::Ignore => {
            debug!(event_type, "GitHub PR event ignored");
        }
    }

    Ok(())
}

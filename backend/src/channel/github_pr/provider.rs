//! GitHub PR channel provider — ChannelProvider trait impl + webhook parsing.

use std::fmt::Write;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::inbound;
use crate::channel::models::{ChannelConfig, ChannelThread, InboundMessage, OutboundReply};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;
use crate::state::AppState;
use crate::task::models::TaskStatus;

use super::api::GithubPrApi;
use super::config::GithubPrProviderConfig;

/// Agent comment marker to filter self-replies.
const AGENT_MARKER: &str = "<!-- moatless-agent -->";

/// Result of parsing a GitHub webhook.
pub enum GithubPrWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// PR was closed/merged — close associated tasks.
    CloseTask { pr_number: u64 },
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
    /// Database pool for looking up provider tokens.
    db: DbPool,
    /// Deduplication cache: delivery_id → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
}

impl GithubPrProvider {
    pub fn new(
        global_webhook_secret: Option<String>,
        global_bot_token: Option<String>,
        db: DbPool,
    ) -> Self {
        Self {
            global_webhook_secret,
            global_bot_token,
            db,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Resolve a GitHub token for API calls.
    ///
    /// Order: effective user's provider_token → global env.
    async fn resolve_token(&self, config: &ChannelConfig) -> Result<String, ChannelError> {
        // 1. Effective user's provider_token for "github"
        //    (run_as_user_id if set, otherwise created_by)
        let user_id = config.effective_user_id();
        let row = sqlx::query_scalar::<_, String>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = 'github' LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| ChannelError::ProviderApi(format!("Failed to fetch provider token: {e}")))?;

        if let Some(token) = row {
            return Ok(token);
        }

        // 2. Global env token
        if let Some(ref token) = self.global_bot_token {
            return Ok(token.clone());
        }

        Err(ChannelError::ProviderApi(format!(
            "No GitHub token available: user {user_id} has no GitHub provider token and GITHUB_CHANNEL_TOKEN env is not set"
        )))
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
            && !provider_config
                .events_filter
                .iter()
                .any(|e| e == event_type)
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

        // Filter events from the run_as_user (prevents moat reply → webhook loop)
        if !sender_login.is_empty() {
            if let Ok(Some(sender_user_id)) = self.resolve_user(sender_login, db).await {
                if sender_user_id == config.effective_user_id() {
                    debug!(sender_login, run_as = %config.effective_user_id(), "Ignoring event from run_as user");
                    return Ok(GithubPrWebhookResult::Ignore);
                }
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
                self.handle_pull_request_event(payload, &ctx, &provider_config)
                    .await
            }
            "issue_comment" => self.handle_issue_comment_event(payload, &ctx).await,
            "pull_request_review_comment" => self.handle_review_comment_event(payload, &ctx).await,
            "pull_request_review" => self.handle_review_event(payload, &ctx).await,
            "check_run" => self.handle_check_run_event(payload, &ctx).await,
            _ => {
                debug!(event_type, "Unsupported GitHub event type");
                Ok(GithubPrWebhookResult::Ignore)
            }
        }
    }

    async fn handle_pull_request_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
        provider_config: &GithubPrProviderConfig,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action == "synchronize" {
            return self.handle_pull_request_sync(payload, ctx).await;
        }

        if action == "closed" {
            if provider_config.close_task_on_close {
                let pr_number = payload
                    .pointer("/pull_request/number")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;
                let merged = payload
                    .pointer("/pull_request/merged")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                info!(pr_number, merged, "PR closed, will close associated tasks");
                return Ok(GithubPrWebhookResult::CloseTask { pr_number });
            }
            debug!("PR closed but close_task_on_close is disabled");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        if action != "opened" && action != "reopened" {
            debug!(action, "Ignoring pull_request action");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr = payload
            .get("pull_request")
            .ok_or_else(|| ChannelError::BadRequest("Missing pull_request in payload".into()))?;

        let pr_number = pr
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;
        let title = pr
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let body = pr
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let html_url = pr
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
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

        if !html_url.is_empty() {
            let _ = writeln!(text, "**Link:** {html_url}");
        }

        if !body.is_empty() {
            let _ = write!(text, "\n### Description\n{body}\n");
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

    /// Handle `pull_request` `synchronize` action (new commits pushed).
    async fn handle_pull_request_sync(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let pr = payload
            .get("pull_request")
            .ok_or_else(|| ChannelError::BadRequest("Missing pull_request in payload".into()))?;

        let pr_number = pr
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ChannelError::BadRequest("Missing PR number".into()))?;
        let html_url = pr
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let head_ref = pr
            .pointer("/head/ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let before = payload
            .get("before")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let after = payload
            .get("after")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let mut text = format!(
            "**@{}** pushed new commits to PR #{pr_number} (`{head_ref}`)\n",
            ctx.sender_login,
        );

        if !html_url.is_empty() {
            let _ = writeln!(text, "{html_url}");
        }

        if !before.is_empty() && !after.is_empty() {
            let short_before = &before[..7.min(before.len())];
            let short_after = &after[..7.min(after.len())];
            let _ = writeln!(text, "\n`{short_before}` → `{short_after}`");
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

        let comment = payload
            .get("comment")
            .ok_or_else(|| ChannelError::BadRequest("Missing comment in payload".into()))?;

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
            text: format!("**@{}** commented:\n\n{}", ctx.sender_login, comment_body),
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

        let comment = payload
            .get("comment")
            .ok_or_else(|| ChannelError::BadRequest("Missing comment in payload".into()))?;

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
        let line = comment.get("line").and_then(serde_json::Value::as_u64);
        let start_line = comment
            .get("start_line")
            .and_then(serde_json::Value::as_u64);
        let html_url = comment
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let in_reply_to_id = comment
            .get("in_reply_to_id")
            .and_then(serde_json::Value::as_u64);

        // Filter comments with agent marker
        if comment_body.contains(AGENT_MARKER) {
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Build rich message text with author, file, line info
        let mut text = format!("**@{}** commented on `{path}`", ctx.sender_login);

        match (start_line, line) {
            (Some(start), Some(end)) if start != end => {
                let _ = write!(text, " (lines {start}-{end})");
            }
            (_, Some(ln)) => {
                let _ = write!(text, " (line {ln})");
            }
            _ => {}
        }
        text.push_str(":\n");

        if !html_url.is_empty() {
            let _ = writeln!(text, "{html_url}");
        }

        if let Some(reply_id) = in_reply_to_id {
            let _ = write!(text, "\n> In reply to comment {reply_id}\n");
        }

        if !diff_hunk.is_empty() {
            let _ = write!(text, "\n```diff\n{diff_hunk}\n```\n");
        }

        let _ = write!(text, "\n{comment_body}");

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
                "in_reply_to_id": in_reply_to_id,
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

        let review = payload
            .get("review")
            .ok_or_else(|| ChannelError::BadRequest("Missing review in payload".into()))?;

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

        let text = format!(
            "**@{}** — {state_label}:\n\n{review_body}",
            ctx.sender_login
        );

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

    /// Failure-class conclusions for check_run events.
    const CHECK_RUN_FAILURE_CONCLUSIONS: &[&str] =
        &["failure", "timed_out", "cancelled", "action_required"];

    /// Maximum length for check_run output summary in the message.
    const CHECK_RUN_SUMMARY_MAX_LEN: usize = 2000;

    /// Handle `check_run` `completed` events with failure conclusions.
    async fn handle_check_run_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "completed" {
            debug!(action, "Ignoring check_run action (not completed)");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let check_run = payload
            .get("check_run")
            .ok_or_else(|| ChannelError::BadRequest("Missing check_run in payload".into()))?;

        let conclusion = check_run
            .get("conclusion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if !Self::CHECK_RUN_FAILURE_CONCLUSIONS.contains(&conclusion) {
            debug!(conclusion, "Ignoring check_run with non-failure conclusion");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Extract PR number — ignore if not PR-related
        let pr_number = check_run
            .pointer("/pull_requests/0/number")
            .and_then(serde_json::Value::as_u64);

        let Some(pr_number) = pr_number else {
            debug!("Ignoring check_run: no associated pull request");
            return Ok(GithubPrWebhookResult::Ignore);
        };

        let name = check_run
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let html_url = check_run
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let head_sha = check_run
            .get("head_sha")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let short_sha = &head_sha[..7.min(head_sha.len())];
        let app_name = check_run
            .pointer("/app/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("CI");

        let output_title = check_run
            .pointer("/output/title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let output_summary = check_run
            .pointer("/output/summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Build rich message
        let mut text = format!(
            "## CI Check Failed: {name}\n\n\
             **Conclusion:** {conclusion}\n\
             **CI:** {app_name}\n\
             **Commit:** `{short_sha}`\n",
        );

        if !html_url.is_empty() {
            let _ = writeln!(text, "**Link:** {html_url}");
        }

        if !output_title.is_empty() || !output_summary.is_empty() {
            text.push_str("\n### Output\n");
            if !output_title.is_empty() {
                let _ = writeln!(text, "**{output_title}**");
            }
            if !output_summary.is_empty() {
                if output_summary.len() > Self::CHECK_RUN_SUMMARY_MAX_LEN {
                    let truncated = &output_summary[..Self::CHECK_RUN_SUMMARY_MAX_LEN];
                    let _ = write!(text, "{truncated}...");
                } else {
                    let _ = write!(text, "{output_summary}");
                }
            }
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
        let token = self.resolve_token(config).await?;
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

        let token = match self.resolve_token(config).await {
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
                c.body.as_deref().is_none_or(|b| !b.contains(AGENT_MARKER))
            })
            .filter(|c| {
                // Exclude comments already delivered to the task
                !exclude_external_ids.contains(&c.id.to_string())
            })
            .map(|c| format!("@{}: {}", c.author(), c.body.as_deref().unwrap_or("")))
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

    info!(
        event_type,
        repo = repo_full_name,
        delivery_id,
        "GitHub PR webhook received"
    );

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
            info!(
                repo = repo_full_name,
                "GitHub PR webhook ignored: config inactive"
            );
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
                info!(
                    repo = repo_full_name,
                    "GitHub PR webhook ignored: no config and no wildcard"
                );
                return Ok(());
            };

            let template_config = GithubPrProviderConfig::from_config(&template);
            if !template_config.auto_create {
                info!(
                    repo = repo_full_name,
                    "GitHub PR webhook ignored: wildcard exists but auto_create is disabled"
                );
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
                    info!(
                        repo = repo_full_name,
                        "GitHub PR webhook ignored: wildcard clone returned None"
                    );
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
        warn!(
            repo = repo_full_name,
            "GitHub webhook has signature but no secret configured, skipping verification"
        );
    }

    // Parse the webhook event
    let result = github_provider
        .parse_webhook(delivery_id, event_type, &payload, &config, &state.db)
        .await?;

    match result {
        GithubPrWebhookResult::Message(msg) => {
            let config_id = config.id.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                // Extract the external_thread_id before process_inbound consumes the msg
                let external_thread_id = msg.thread_id.clone();
                let msg_clone_for_fanout = (*msg).clone();

                match inbound::process_inbound(&state_clone, *msg, None).await {
                    Ok(primary_task_id) => {
                        // Fan out to additional tasks connected to the same PR
                        if let Some(ref ext_tid) = external_thread_id {
                            super::super::listen::deliver_to_connected_threads(
                                &state_clone,
                                &config_id,
                                ext_tid,
                                &primary_task_id,
                                &msg_clone_for_fanout,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to process GitHub PR inbound message");
                    }
                }
            });
        }
        GithubPrWebhookResult::CloseTask { pr_number } => {
            let state_clone = state.clone();
            let config_id = config.id.clone();
            tokio::spawn(async move {
                close_tasks_for_pr(&state_clone, &config_id, pr_number).await;
            });
        }
        GithubPrWebhookResult::Ignore => {
            debug!(event_type, "GitHub PR event ignored");
        }
    }

    Ok(())
}

/// Close all tasks associated with a PR thread.
async fn close_tasks_for_pr(state: &AppState, config_id: &str, pr_number: u64) {
    let threads = match state
        .channel_threads
        .find_all_by_config_and_thread(config_id, &pr_number.to_string())
        .await
    {
        Ok(t) => t,
        Err(e) => {
            warn!(config_id, pr_number, error = %e, "Failed to look up threads for PR close");
            return;
        }
    };

    for thread in threads {
        let task_id = &thread.task_id;

        // Skip already-closed tasks
        match state.tasks.find_by_id_internal(task_id).await {
            Ok(Some(task)) if task.status == TaskStatus::Closed => {
                debug!(task_id, pr_number, "Task already closed, skipping");
                continue;
            }
            Ok(None) => {
                warn!(task_id, pr_number, "Task not found for PR close");
                continue;
            }
            Err(e) => {
                warn!(task_id, pr_number, error = %e, "Failed to fetch task for PR close");
                continue;
            }
            _ => {}
        }

        if let Err(e) = state.sandbox.cleanup(task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on PR close");
        }

        if let Err(e) = state
            .tasks
            .update_task(
                task_id,
                &crate::task::dao::TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
            )
            .await
        {
            warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
            continue;
        }

        if let Some(ref events) = state.events {
            events
                .publish_task_event(
                    task_id,
                    moatless_events::TaskSubType::Closed,
                    Some("claude-code"),
                    serde_json::json!({"reason": "pr_closed", "pr_number": pr_number}),
                )
                .await;
        }

        info!(task_id = %task_id, pr_number, "Task closed due to PR close");
    }
}

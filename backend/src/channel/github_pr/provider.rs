//! GitHub PR channel provider — ChannelProvider trait impl + webhook parsing.

use std::fmt::Write;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::channel::error::ChannelError;
use crate::channel::external_id;
use crate::channel::models::{
    ChannelConfig, ChannelThread, InboundMessage, IngressRule, OutboundReply, ProviderConnection,
    TaskSubscription,
};
use crate::channel::provider::{ChannelProvider, ChannelType};
use crate::db::DbPool;
use crate::state::AppState;

use super::api::GithubPrApi;
use super::config::GithubPrRuleConfig;

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
    rule: &'a IngressRule,
    owner: &'a str,
    repo: &'a str,
    sender_login: &'a str,
    db: &'a DbPool,
}

/// GitHub PR channel provider.
pub struct GithubPrProvider {
    /// Global webhook secret from env (`GITHUB_CHANNEL_WEBHOOK_SECRET`).
    /// Used as a fallback when the org connection has no `webhook_secret` set.
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

    /// Resolve a GitHub token for API calls keyed by user id.
    ///
    /// Order: provider_tokens(user_id) → global env.
    async fn resolve_token_for_user(&self, user_id: &str) -> Result<String, ChannelError> {
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

        if let Some(ref token) = self.global_bot_token {
            return Ok(token.clone());
        }

        Err(ChannelError::ProviderApi(format!(
            "No GitHub token available: user {user_id} has no GitHub provider token and GITHUB_CHANNEL_TOKEN env is not set"
        )))
    }

    /// Extract owner, repo, and PR number from a `ChannelThread`'s properties.
    /// Used by the synthetic-thread path in `runtime.rs::deliver_to_subscription`.
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

    /// Extract owner, repo, pr_number from a `TaskSubscription`'s `reply_target`.
    fn parse_subscription_target(
        subscription: &TaskSubscription,
    ) -> Result<(String, String, u64), ChannelError> {
        let owner = subscription
            .reply_target
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChannelError::Internal("Missing owner in subscription reply_target".into()))?
            .to_string();
        let repo = subscription
            .reply_target
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChannelError::Internal("Missing repo in subscription reply_target".into()))?
            .to_string();
        let pr_number = subscription
            .reply_target
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ChannelError::Internal("Missing pr_number in subscription reply_target".into())
            })?;
        Ok((owner, repo, pr_number))
    }

    /// Parse a GitHub webhook into an `InboundMessage`.
    pub async fn parse_webhook(
        &self,
        delivery_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
        rule: &IngressRule,
        db: &DbPool,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let rule_config = GithubPrRuleConfig::from_rule(rule);

        // Dedup by delivery ID
        if self.dedup_cache.get(delivery_id).await.is_some() {
            debug!(delivery_id, "GitHub webhook deduplicated");
            return Ok(GithubPrWebhookResult::Ignore);
        }
        self.dedup_cache.insert(delivery_id.to_string(), ()).await;

        // Filter by events_filter config
        if !rule_config.events_filter.is_empty()
            && !rule_config
                .events_filter
                .iter()
                .any(|e| e == event_type)
        {
            debug!(event_type, "GitHub event filtered out by ingress rule");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Extract sender login (used by handlers for message text + user resolution).
        let sender_login = payload
            .pointer("/sender/login")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // (Per-config bot_username self-filter dropped. Self-echo is now handled
        // per-task in `send_message_flow` via `has_channel_external_id`, so sibling
        // watcher tasks connected to the same PR still receive events.)

        let repo_full_name = payload
            .pointer("/repository/full_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let (owner, repo) = repo_full_name
            .split_once('/')
            .ok_or_else(|| ChannelError::BadRequest("Missing repository.full_name".into()))?;

        let ctx = EventContext {
            rule,
            owner,
            repo,
            sender_login,
            db,
        };

        match event_type {
            "pull_request" => self.handle_pull_request_event(payload, &ctx, &rule_config).await,
            "issue_comment" => self.handle_issue_comment_event(payload, &ctx).await,
            "pull_request_review_comment" => self.handle_review_comment_event(payload, &ctx).await,
            "pull_request_review" => self.handle_review_event(payload, &ctx).await,
            "check_run" => self.handle_check_run_event(payload, &ctx).await,
            "check_suite" => {
                self.handle_check_suite_event(payload, &ctx, &rule_config).await
            }
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
        rule_config: &GithubPrRuleConfig,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action == "synchronize" {
            return self.handle_pull_request_sync(payload, ctx).await;
        }

        if action == "closed" {
            if rule_config.close_task_on_close {
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
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
            thread_id: None,
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
            channel_external_id: Some(external_id::github_pr_event(action, pr_number, None)),
        })))
    }

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
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        let sync_sha = if after.is_empty() { None } else { Some(after) };
        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_pr_event(
                "synchronize",
                pr_number,
                sync_sha,
            )),
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

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_issue_comment(comment_id)),
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
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_review_comment(comment_id)),
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
        let review_id = review
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        if review_body.is_empty() {
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
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_review(review_id)),
        })))
    }

    /// Failure-class conclusions for check_run events.
    const CHECK_RUN_FAILURE_CONCLUSIONS: &[&str] =
        &["failure", "timed_out", "cancelled", "action_required"];

    /// Conclusion value indicating a successful `check_suite`.
    const CHECK_SUITE_SUCCESS_CONCLUSION: &str = "success";

    /// Maximum length for check_run output summary in the message.
    const CHECK_RUN_SUMMARY_MAX_LEN: usize = 2000;

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

        let pr_number = check_run
            .pointer("/pull_requests/0/number")
            .and_then(serde_json::Value::as_u64);

        let Some(pr_number) = pr_number else {
            debug!("Ignoring check_run: no associated pull request");
            return Ok(GithubPrWebhookResult::Ignore);
        };

        let check_run_id = check_run
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
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
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_check_run(check_run_id)),
        })))
    }

    async fn handle_check_suite_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
        rule_config: &GithubPrRuleConfig,
    ) -> Result<GithubPrWebhookResult, ChannelError> {
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if action != "completed" {
            debug!(action, "Ignoring check_suite action (not completed)");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        if !rule_config.notify_on_success {
            debug!("Ignoring check_suite: notify_on_success disabled");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let check_suite = payload
            .get("check_suite")
            .ok_or_else(|| ChannelError::BadRequest("Missing check_suite in payload".into()))?;

        let conclusion = check_suite
            .get("conclusion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if conclusion != Self::CHECK_SUITE_SUCCESS_CONCLUSION {
            debug!(
                conclusion,
                "Ignoring check_suite with non-success conclusion"
            );
            return Ok(GithubPrWebhookResult::Ignore);
        }

        let pr_number = check_suite
            .pointer("/pull_requests/0/number")
            .and_then(serde_json::Value::as_u64);

        let Some(pr_number) = pr_number else {
            debug!("Ignoring check_suite: no associated pull request");
            return Ok(GithubPrWebhookResult::Ignore);
        };

        let check_suite_id = check_suite
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let head_sha = check_suite
            .get("head_sha")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let short_sha = &head_sha[..7.min(head_sha.len())];
        let app_name = check_suite
            .pointer("/app/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("CI");

        let text = format!(
            "## CI Passed\n\n\
             **CI:** {app_name}\n\
             **Commit:** `{short_sha}`\n",
        );

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: ChannelType::GithubPr,
            source_id: ctx.rule.scope_matcher.clone(),
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
            channel_external_id: Some(external_id::github_check_suite(check_suite_id)),
        })))
    }

    /// Resolve a GitHub login to a system user, falling back to the rule's
    /// effective user (run_as_user_id ?? created_by).
    async fn resolve_user_or_fallback(
        &self,
        github_login: &str,
        rule: &IngressRule,
        db: &DbPool,
    ) -> String {
        if let Ok(Some(user_id)) = self.resolve_user(github_login, db).await {
            user_id
        } else {
            debug!(
                github_login,
                fallback = %rule.effective_user_id(),
                "GitHub user not mapped, using rule owner"
            );
            rule.effective_user_id().to_string()
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

    /// GitHub allows up to 65536 chars per issue/PR comment. Leave headroom for
    /// the agent marker and the optional task-URL footer that
    /// `send_reply_for_subscription` prepends/appends.
    fn max_message_length(&self) -> usize {
        65_000
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, ChannelError> {
        let user_id = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM provider_tokens WHERE provider = 'github' AND provider_login = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(ChannelError::Db)?;

        Ok(user_id)
    }

    /// Legacy thread-based outbound is not supported for GitHub. Outbound
    /// goes through `send_reply_for_subscription` instead.
    async fn send_reply(
        &self,
        _thread: &ChannelThread,
        _config: &ChannelConfig,
        _message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        Err(ChannelError::BadRequest(
            "GitHub provider uses subscription-based outbound; \
             call send_reply_for_subscription"
                .into(),
        ))
    }

    async fn send_reply_for_subscription(
        &self,
        subscription: &TaskSubscription,
        _connection: &ProviderConnection,
        rule: Option<&IngressRule>,
        message: &OutboundReply,
    ) -> Result<Option<String>, ChannelError> {
        let (owner, repo, pr_number) = Self::parse_subscription_target(subscription)?;
        let user_id = match rule {
            Some(r) => r.effective_user_id().to_string(),
            None => subscription
                .metadata
                .get("filter_user_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        if user_id.is_empty() {
            return Err(ChannelError::ProviderApi(
                "Cannot resolve GitHub token: no ingress rule attached to subscription".into(),
            ));
        }
        let token = self.resolve_token_for_user(&user_id).await?;
        let api = GithubPrApi::new(&token);

        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }

        let comment_id = api.post_comment(&owner, &repo, pr_number, &body).await?;
        Ok(Some(external_id::github_issue_comment(comment_id)))
    }

    async fn fetch_context(
        &self,
        thread: &ChannelThread,
        config: &ChannelConfig,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, ChannelError> {
        let (owner, repo, pr_number) = Self::parse_thread_props(thread)?;

        let user_id = config.effective_user_id();
        let token = match self.resolve_token_for_user(user_id).await {
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
                c.body.as_deref().is_none_or(|b| !b.contains(AGENT_MARKER))
            })
            .filter(|c| {
                let prefixed = external_id::github_issue_comment(c.id);
                let bare = c.id.to_string();
                !exclude_external_ids.contains(&prefixed) && !exclude_external_ids.contains(&bare)
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
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event_type.is_empty() {
        return Err(ChannelError::BadRequest(
            "Missing X-GitHub-Event header".into(),
        ));
    }

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
        "GitHub webhook received"
    );

    let owner_login = payload
        .pointer("/repository/owner/login")
        .and_then(serde_json::Value::as_str)
        .or_else(|| repo_full_name.split('/').next())
        .ok_or_else(|| ChannelError::BadRequest("Missing repository.owner.login".into()))?;

    let resolved = super::runtime::resolve_ingress(state, owner_login, repo_full_name).await?;
    let Some((connection, rule)) = resolved else {
        info!(
            repo = repo_full_name,
            owner = owner_login,
            "GitHub webhook ignored: no provider_connection / ingress_rule"
        );
        return Ok(());
    };

    if !rule.active {
        info!(
            repo = repo_full_name,
            "GitHub webhook ignored: ingress rule inactive"
        );
        return Ok(());
    }

    // Verify webhook signature against the org connection's secret.
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let secret = connection
        .webhook_secret
        .as_deref()
        .or(github_provider.global_webhook_secret.as_deref());

    if let Some(secret) = secret {
        super::signature::verify_signature(secret, body, signature)?;
    } else if !signature.is_empty() {
        warn!(
            repo = repo_full_name,
            "GitHub webhook has signature but no secret configured on connection, refusing"
        );
        return Err(ChannelError::BadRequest(
            "Webhook signature provided but no secret configured for connection".into(),
        ));
    }

    let result = github_provider
        .parse_webhook(delivery_id, event_type, &payload, &rule, &state.db)
        .await?;

    match result {
        GithubPrWebhookResult::Message(msg) => {
            let connection = connection.clone();
            let rule = rule.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    super::runtime::route_inbound_message(&state_clone, &connection, &rule, *msg)
                        .await
                {
                    warn!(error = %e, "Failed to route GitHub PR inbound message");
                }
            });
        }
        GithubPrWebhookResult::CloseTask { pr_number } => {
            let rule = rule.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    super::runtime::close_tasks_for_pr(&state_clone, &rule, pr_number).await
                {
                    warn!(error = %e, "Failed to close GitHub PR tasks");
                }
            });
        }
        GithubPrWebhookResult::Ignore => {
            debug!(event_type, "GitHub PR event ignored");
        }
    }

    Ok(())
}

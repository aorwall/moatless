//! GitHub PR adapter — Adapter trait impl + webhook parsing.

use std::fmt::Write;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, InboundMessage, OutboundReply, TaskBinding,
    TaskTriggerRoute, TriggerContext,
};
use crate::adapters::{Adapter, AdapterKind, BindingDestination};
use crate::db::DbPool;
use crate::state::AppState;

use super::api::GithubPrApi;
use super::config::GithubPrRuleConfig;

/// Agent comment marker to filter self-replies.
const AGENT_MARKER: &str = "<!-- moatless-agent -->";

/// Map a user-supplied emoji name to GitHub's reaction content enum.
///
/// GitHub accepts only these eight values:
/// `+1`, `-1`, `laugh`, `confused`, `heart`, `hooray`, `rocket`, `eyes`.
/// We accept the canonical names plus a few common aliases.
pub(crate) fn to_github_content(emoji: &str) -> Result<&'static str, AdapterError> {
    let normalized = emoji.trim().trim_matches(':').to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "+1" | "thumbsup" | "thumbs_up" => "+1",
        "-1" | "thumbsdown" | "thumbs_down" => "-1",
        "laugh" | "laughing" | "smile" => "laugh",
        "confused" | "thinking" | "thinking_face" => "confused",
        "heart" | "love" | "red_heart" => "heart",
        "hooray" | "tada" | "party" => "hooray",
        "rocket" => "rocket",
        "eyes" | "looking" => "eyes",
        other => {
            return Err(AdapterError::BadRequest(format!(
                "GitHub does not accept emoji `{other}` (allowed: +1, -1, laugh, confused, heart, hooray, rocket, eyes)"
            )));
        }
    };
    Ok(mapped)
}

/// Result of parsing a GitHub webhook.
pub enum GithubPrWebhookResult {
    /// Parsed message ready for routing.
    Message(Box<InboundMessage>),
    /// PR was closed/merged — close associated tasks.
    CloseTask(PrCloseInfo),
    /// Webhook should be ignored (unsupported event, bot self-reply, etc.).
    Ignore,
}

/// Metadata about a closed PR, captured at parse time and used downstream
/// to compose the auto-close chat notice.
#[derive(Debug, Clone)]
pub struct PrCloseInfo {
    pub pr_number: u64,
    pub merged: bool,
    pub title: String,
    pub html_url: String,
    pub sender_login: String,
}

/// Context passed through event handlers to avoid too many function arguments.
struct EventContext<'a> {
    rule: &'a TaskTriggerRoute,
    owner: &'a str,
    repo: &'a str,
    sender_login: &'a str,
    db: &'a DbPool,
}

/// GitHub PR adapter.
pub struct GithubPrAdapter {
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

impl GithubPrAdapter {
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
    async fn resolve_token_for_user(&self, user_id: &str) -> Result<String, AdapterError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = 'github' LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| AdapterError::AdapterApi(format!("Failed to fetch provider token: {e}")))?;

        if let Some(token) = row {
            return Ok(token);
        }

        if let Some(ref token) = self.global_bot_token {
            return Ok(token.clone());
        }

        Err(AdapterError::AdapterApi(format!(
            "No GitHub token available: user {user_id} has no GitHub provider token and GITHUB_CHANNEL_TOKEN env is not set"
        )))
    }

    /// Extract owner, repo, and PR number from a `BindingContext`'s properties.
    /// Used by the synthetic-thread path in `runtime.rs::deliver_to_binding`.
    fn parse_thread_props(thread: &BindingContext) -> Result<(String, String, u64), AdapterError> {
        let owner = thread
            .destination_config
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AdapterError::Internal("Missing owner in thread properties".into()))?
            .to_string();
        let repo = thread
            .destination_config
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AdapterError::Internal("Missing repo in thread properties".into()))?
            .to_string();
        let pr_number = thread
            .destination_config
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                AdapterError::Internal("Missing pr_number in thread properties".into())
            })?;
        Ok((owner, repo, pr_number))
    }

    /// Extract owner, repo from a `BindingContext` (ignores pr_number — used by
    /// reaction routes where the comment id alone determines the target URL).
    fn parse_owner_repo(thread: &BindingContext) -> Result<(String, String), AdapterError> {
        let owner = thread
            .destination_config
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest("Missing owner in binding destination_config".into())
            })?
            .to_string();
        let repo = thread
            .destination_config
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest("Missing repo in binding destination_config".into())
            })?
            .to_string();
        Ok((owner, repo))
    }

    /// Resolve a GitHub token for reactions: prefer the binding's
    /// `filter_user_id` (the task owner), then global env. Returns the resolved
    /// token plus the user id used (for logging).
    async fn resolve_reaction_token(
        &self,
        binding: &BindingContext,
    ) -> Result<String, AdapterError> {
        let user_id = binding
            .destination_config
            .get("filter_user_id")
            .or_else(|| binding.metadata.get("filter_user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !user_id.is_empty() {
            return self.resolve_token_for_user(user_id).await;
        }
        self.global_bot_token.clone().ok_or_else(|| {
            AdapterError::AdapterApi(
                "No GitHub token available for reactions: binding has no filter_user_id and \
                 GITHUB_CHANNEL_TOKEN env is not set"
                    .into(),
            )
        })
    }

    /// Extract owner, repo, pr_number from a `TaskBinding`'s `destination_config`.
    fn parse_binding_target(binding: &TaskBinding) -> Result<(String, String, u64), AdapterError> {
        let owner = binding
            .destination_config
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::Internal("Missing owner in binding destination_config".into())
            })?
            .to_string();
        let repo = binding
            .destination_config
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::Internal("Missing repo in binding destination_config".into())
            })?
            .to_string();
        let pr_number = binding
            .destination_config
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                AdapterError::Internal("Missing pr_number in binding destination_config".into())
            })?;
        Ok((owner, repo, pr_number))
    }

    /// Parse a GitHub webhook into an `InboundMessage`.
    pub async fn parse_webhook(
        &self,
        delivery_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
        rule: &TaskTriggerRoute,
        db: &DbPool,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
        let rule_config = GithubPrRuleConfig::from_rule(rule);

        // Dedup by delivery ID
        if self.dedup_cache.get(delivery_id).await.is_some() {
            debug!(delivery_id, "GitHub webhook deduplicated");
            return Ok(GithubPrWebhookResult::Ignore);
        }
        self.dedup_cache.insert(delivery_id.to_string(), ()).await;

        // Filter by events_filter config
        if !rule_config.events_filter.is_empty()
            && !rule_config.events_filter.iter().any(|e| e == event_type)
        {
            debug!(event_type, "GitHub event filtered out by task trigger");
            return Ok(GithubPrWebhookResult::Ignore);
        }

        // Extract sender login (used by handlers for message text + user resolution).
        let sender_login = payload
            .pointer("/sender/login")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // (Per-config bot_username self-filter dropped. Self-echo is now handled
        // per-task in `send_message_flow` via `has_external_event_id`, so sibling
        // watcher tasks connected to the same PR still receive events.)

        let repo_full_name = payload
            .pointer("/repository/full_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let (owner, repo) = repo_full_name
            .split_once('/')
            .ok_or_else(|| AdapterError::BadRequest("Missing repository.full_name".into()))?;

        let ctx = EventContext {
            rule,
            owner,
            repo,
            sender_login,
            db,
        };

        match event_type {
            "pull_request" => {
                self.handle_pull_request_event(payload, &ctx, &rule_config)
                    .await
            }
            "issue_comment" => self.handle_issue_comment_event(payload, &ctx).await,
            "pull_request_review_comment" => self.handle_review_comment_event(payload, &ctx).await,
            "pull_request_review" => self.handle_review_event(payload, &ctx).await,
            "check_run" => self.handle_check_run_event(payload, &ctx).await,
            "check_suite" => {
                self.handle_check_suite_event(payload, &ctx, &rule_config)
                    .await
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
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
                    .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;
                let merged = payload
                    .pointer("/pull_request/merged")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let title = payload
                    .pointer("/pull_request/title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let html_url = payload
                    .pointer("/pull_request/html_url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                info!(pr_number, merged, "PR closed, will close associated tasks");
                return Ok(GithubPrWebhookResult::CloseTask(PrCloseInfo {
                    pr_number,
                    merged,
                    title,
                    html_url,
                    sender_login: ctx.sender_login.to_string(),
                }));
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
            .ok_or_else(|| AdapterError::BadRequest("Missing pull_request in payload".into()))?;

        let pr_number = pr
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;
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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::pr_event(action, pr_number, None)),
        })))
    }

    async fn handle_pull_request_sync(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
        let pr = payload
            .get("pull_request")
            .ok_or_else(|| AdapterError::BadRequest("Missing pull_request in payload".into()))?;

        let pr_number = pr
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;
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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::pr_event(
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
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
            .ok_or_else(|| AdapterError::BadRequest("Missing issue number".into()))?;

        let comment = payload
            .get("comment")
            .ok_or_else(|| AdapterError::BadRequest("Missing comment in payload".into()))?;

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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::issue_comment(comment_id)),
        })))
    }

    async fn handle_review_comment_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
            .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;

        let comment = payload
            .get("comment")
            .ok_or_else(|| AdapterError::BadRequest("Missing comment in payload".into()))?;

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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::review_comment(comment_id)),
        })))
    }

    async fn handle_review_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
            .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;

        let review = payload
            .get("review")
            .ok_or_else(|| AdapterError::BadRequest("Missing review in payload".into()))?;

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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::review(review_id)),
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
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
            .ok_or_else(|| AdapterError::BadRequest("Missing check_run in payload".into()))?;

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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::check_run(check_run_id)),
        })))
    }

    async fn handle_check_suite_event(
        &self,
        payload: &serde_json::Value,
        ctx: &EventContext<'_>,
        rule_config: &GithubPrRuleConfig,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
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
            .ok_or_else(|| AdapterError::BadRequest("Missing check_suite in payload".into()))?;

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
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
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
            external_event_id: Some(super::external_id::check_suite(check_suite_id)),
        })))
    }

    /// Resolve a GitHub login to a system user, falling back to the rule's
    /// effective user (run_as_user_id ?? created_by).
    async fn resolve_user_or_fallback(
        &self,
        github_login: &str,
        rule: &TaskTriggerRoute,
        db: &DbPool,
    ) -> String {
        resolve_user_or_fallback(github_login, rule, db).await
    }
}

/// Resolve a GitHub login to a system user id via `provider_tokens`,
/// falling back to the rule's effective user (`run_as_user_id ?? created_by`).
///
/// Free-function form so callers without a `GithubPrAdapter` instance
/// (e.g. `runtime::close_tasks_for_pr`) can use the same lookup.
pub(crate) async fn resolve_user_or_fallback(
    github_login: &str,
    rule: &TaskTriggerRoute,
    db: &DbPool,
) -> String {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM provider_tokens WHERE provider = 'github' AND provider_login = $1 LIMIT 1",
    )
    .bind(github_login)
    .fetch_optional(db)
    .await;

    if let Ok(Some(user_id)) = row {
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

#[async_trait::async_trait]
impl Adapter for GithubPrAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::GithubPr
    }

    fn provider_key(&self) -> &'static str {
        "github_pr"
    }

    fn connection_provider_key(&self) -> &'static str {
        "github"
    }

    fn connection_scope_value_from_existing_thread(
        &self,
        source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, AdapterError> {
        source_id
            .split('/')
            .next()
            .filter(|owner| !owner.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                AdapterError::BadRequest(format!(
                    "Invalid GitHub source_id (expected owner/repo): {source_id}"
                ))
            })
    }

    fn allow_default_connection_fallback(&self) -> bool {
        false
    }

    fn allow_wildcard_trigger_fallback(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// GitHub allows up to 65536 chars per issue/PR comment. Leave headroom for
    /// the agent marker and the optional task-URL footer that
    /// `send_reply_for_binding` prepends/appends.
    fn max_message_length(&self) -> usize {
        65_000
    }

    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        let user_id = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM provider_tokens WHERE provider = 'github' AND provider_login = $1 LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(db)
        .await
        .map_err(AdapterError::Db)?;

        Ok(user_id)
    }

    fn binding_destination_from_existing_thread(
        &self,
        _source_id: &str,
        _external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        let owner = thread_properties
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest("thread_properties missing required owner".into())
            })?;
        let repo = thread_properties
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AdapterError::BadRequest("thread_properties missing required repo".into())
            })?;
        let pr_number = thread_properties
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                AdapterError::BadRequest("thread_properties missing required pr_number".into())
            })?;
        let mut destination_config = serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr_number": pr_number,
        });
        if let Some(filter_user_id) = filter_user_id {
            destination_config["filter_user_id"] = serde_json::json!(filter_user_id);
        }

        Ok(BindingDestination::new(
            "github_pr",
            format!("{owner}/{repo}#{pr_number}"),
            destination_config,
        ))
    }

    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        super::external_id::aliases(external_id)
    }

    /// Legacy thread-based outbound is not supported for GitHub. Outbound
    /// goes through `send_reply_for_binding` instead.
    async fn send_reply(
        &self,
        _thread: &BindingContext,
        _config: &TriggerContext,
        _message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        Err(AdapterError::BadRequest(
            "GitHub provider uses binding-based outbound; \
             call send_reply_for_binding"
                .into(),
        ))
    }

    async fn send_reply_for_binding(
        &self,
        binding: &TaskBinding,
        _connection: &AdapterConnection,
        rule: Option<&TaskTriggerRoute>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let (owner, repo, pr_number) = Self::parse_binding_target(binding)?;
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
                "Cannot resolve GitHub token: no task trigger attached to binding".into(),
            ));
        }
        let token = self.resolve_token_for_user(&user_id).await?;
        let api = GithubPrApi::new(&token);

        let mut body = format!("{AGENT_MARKER}\n{}", message.text);
        if let Some(ref url) = message.task_url {
            let _ = write!(body, "\n\n---\n[View task]({url})");
        }

        let comment_id = api.post_comment(&owner, &repo, pr_number, &body).await?;
        Ok(Some(super::external_id::issue_comment(comment_id)))
    }

    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&TaskTriggerRoute>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        // Only PR review comments support threaded replies. Top-level issue
        // comments fall back to the binding-level reply (a new top-level
        // PR comment).
        match super::external_id::comment_target_from_id(target_external_id) {
            Some(super::external_id::CommentTarget::Review(comment_id)) => {
                let (owner, repo, pr_number) = Self::parse_binding_target(binding)?;
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
                        "Cannot resolve GitHub token: no task trigger attached to binding".into(),
                    ));
                }
                let token = self.resolve_token_for_user(&user_id).await?;
                let api = GithubPrApi::new(&token);

                let mut body = format!("{AGENT_MARKER}\n{}", message.text);
                if let Some(ref url) = message.task_url {
                    let _ = write!(body, "\n\n---\n[View task]({url})");
                }

                let new_id = api
                    .reply_to_review_comment(&owner, &repo, pr_number, comment_id, &body)
                    .await?;
                Ok(Some(super::external_id::review_comment(new_id)))
            }
            _ => {
                self.send_reply_for_binding(binding, connection, rule, message)
                    .await
            }
        }
    }

    async fn add_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let target = super::external_id::comment_target_from_id(target_external_id).ok_or_else(
            || {
                AdapterError::BadRequest(format!(
                    "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
                ))
            },
        )?;
        let content = to_github_content(emoji)?;
        let (owner, repo) = Self::parse_owner_repo(binding)?;
        let token = self.resolve_reaction_token(binding).await?;
        let api = GithubPrApi::new(&token);
        let reaction_id = match target {
            super::external_id::CommentTarget::Issue(id) => {
                api.create_issue_comment_reaction(&owner, &repo, id, content)
                    .await?
            }
            super::external_id::CommentTarget::Review(id) => {
                api.create_review_comment_reaction(&owner, &repo, id, content)
                    .await?
            }
        };
        Ok(Some(reaction_id.to_string()))
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<(), AdapterError> {
        let target = super::external_id::comment_target_from_id(target_external_id).ok_or_else(
            || {
                AdapterError::BadRequest(format!(
                    "GitHub reaction target `{target_external_id}` must be a github:ic:<id> or github:rc:<id> id"
                ))
            },
        )?;
        let content = to_github_content(emoji)?;
        let (owner, repo) = Self::parse_owner_repo(binding)?;
        let token = self.resolve_reaction_token(binding).await?;
        let api = GithubPrApi::new(&token);
        let bot_login = api.authenticated_login().await?;
        let (reactions, comment_id, is_review) = match target {
            super::external_id::CommentTarget::Issue(id) => (
                api.list_issue_comment_reactions(&owner, &repo, id).await?,
                id,
                false,
            ),
            super::external_id::CommentTarget::Review(id) => (
                api.list_review_comment_reactions(&owner, &repo, id).await?,
                id,
                true,
            ),
        };
        let reaction = reactions
            .into_iter()
            .find(|r| {
                r.content == content
                    && r.user.as_ref().is_some_and(|u| u.login == bot_login)
            })
            .ok_or_else(|| {
                AdapterError::NotFound(format!(
                    "GitHub: no `{content}` reaction by `{bot_login}` to remove on comment {comment_id}"
                ))
            })?;
        if is_review {
            api.delete_review_comment_reaction(&owner, &repo, comment_id, reaction.id)
                .await?;
        } else {
            api.delete_issue_comment_reaction(&owner, &repo, comment_id, reaction.id)
                .await?;
        }
        Ok(())
    }

    async fn fetch_context(
        &self,
        thread: &BindingContext,
        config: &TriggerContext,
        limit: usize,
        exclude_external_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
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
            .filter(|c| c.body.as_deref().is_none_or(|b| !b.contains(AGENT_MARKER)))
            .filter(|c| {
                let prefixed = super::external_id::issue_comment(c.id);
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
    github_provider: &GithubPrAdapter,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> Result<(), AdapterError> {
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event_type.is_empty() {
        return Err(AdapterError::BadRequest(
            "Missing X-GitHub-Event header".into(),
        ));
    }

    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

    let repo_full_name = payload
        .pointer("/repository/full_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AdapterError::BadRequest("Missing repository.full_name".into()))?;

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
        .ok_or_else(|| AdapterError::BadRequest("Missing repository.owner.login".into()))?;

    let resolved =
        super::runtime::resolve_provider_event(state, owner_login, repo_full_name).await?;
    let Some((connection, rule)) = resolved else {
        info!(
            repo = repo_full_name,
            owner = owner_login,
            "GitHub webhook ignored: no provider_connection / task_trigger"
        );
        return Ok(());
    };

    if !rule.active {
        info!(
            repo = repo_full_name,
            "GitHub webhook ignored: task trigger inactive"
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
        return Err(AdapterError::BadRequest(
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
        GithubPrWebhookResult::CloseTask(info) => {
            let connection = connection.clone();
            let rule = rule.clone();
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    super::runtime::close_tasks_for_pr(&state_clone, &connection, &rule, &info)
                        .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::Adapter;

    #[tokio::test]
    async fn provider_keys_separate_channel_and_integration_storage() {
        let provider = GithubPrAdapter::new(None, None, fake_db_pool());
        assert_eq!(provider.provider_key(), "github_pr");
        assert_eq!(provider.connection_provider_key(), "github");
        assert!(!provider.allow_default_connection_fallback());
        assert!(!provider.allow_wildcard_trigger_fallback());
    }

    #[tokio::test]
    async fn binding_destination_from_existing_thread_builds_pr_subject() {
        let provider = GithubPrAdapter::new(None, None, fake_db_pool());
        let destination = provider
            .binding_destination_from_existing_thread(
                "foo/bar",
                "",
                &serde_json::json!({
                    "owner": "foo",
                    "repo": "bar",
                    "pr_number": 42,
                }),
                Some("user-1"),
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "github_pr");
        assert_eq!(destination.subject_key, "foo/bar#42");
        assert_eq!(destination.destination_config["owner"], "foo");
        assert_eq!(destination.destination_config["repo"], "bar");
        assert_eq!(destination.destination_config["pr_number"], 42);
        assert_eq!(destination.destination_config["filter_user_id"], "user-1");
        assert_eq!(destination.last_external_event_id, None);
    }

    #[test]
    fn to_github_content_accepts_canonical_and_aliases() {
        assert_eq!(to_github_content("+1").unwrap(), "+1");
        assert_eq!(to_github_content("thumbsup").unwrap(), "+1");
        assert_eq!(to_github_content(":thumbsup:").unwrap(), "+1");
        assert_eq!(to_github_content("THUMBSDOWN").unwrap(), "-1");
        assert_eq!(to_github_content("rocket").unwrap(), "rocket");
        assert_eq!(to_github_content("tada").unwrap(), "hooray");
        assert_eq!(to_github_content("eyes").unwrap(), "eyes");
    }

    #[test]
    fn to_github_content_rejects_unknown_names() {
        let err = to_github_content("celebration").unwrap_err();
        assert!(matches!(err, AdapterError::BadRequest(_)));
    }

    #[tokio::test]
    async fn external_id_aliases_include_legacy_comment_ids_only() {
        let provider = GithubPrAdapter::new(None, None, fake_db_pool());
        assert_eq!(
            provider.external_id_aliases("github:ic:42"),
            vec!["github:ic:42".to_string(), "42".to_string()]
        );
        assert_eq!(
            provider.external_id_aliases("github:rc:99"),
            vec!["github:rc:99".to_string(), "99".to_string()]
        );
        assert_eq!(
            provider.external_id_aliases("github:pr:opened:7"),
            vec!["github:pr:opened:7".to_string()]
        );
    }

    fn fake_db_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }
}

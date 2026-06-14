//! GitHub PR adapter — Adapter trait impl + webhook parsing.

use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, BindingContext, InboundMessage, OutboundReply, TaskBinding,
    TaskTriggerRoute, TriggerContext,
};
use crate::adapters::{Adapter, AdapterKind, BindingDestination};
use crate::auth::credentials::resolve_user_github_token;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::db::DbPool;
use crate::state::AppState;

use super::api::GithubPrApi;
use super::config::GithubPrRuleConfig;

/// Agent comment marker prefix. Outbound bodies are tagged with the source
/// task id (`<!-- moatless-agent:TASK_ID -->`) so we can drop only the
/// authoring task's own echoes on the way back in, while letting other tasks
/// on the same PR receive the comment as a user message.
const MARKER_PREFIX: &str = "<!-- moatless-agent";

/// Build the outbound agent marker for a given task id.
fn agent_marker(task_id: &str) -> String {
    format!("<!-- moatless-agent:{task_id} -->")
}

/// Classification of a leading agent marker on a comment body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentMarker {
    /// Legacy bare marker `<!-- moatless-agent -->` — no task id embedded.
    /// Predates per-task tagging; still ignored on inbound for backward compat.
    Bare,
    /// New tagged marker `<!-- moatless-agent:TASK_ID -->`.
    Tagged(String),
}

/// Parse the leading agent marker (if any) from a comment body.
///
/// Matches only at `body.trim_start()`. Returns `None` when the body does not
/// begin with the marker prefix; `Some(Bare)` for legacy bare markers or any
/// malformed tagged form (e.g. empty id) — these are treated as
/// "agent-authored, source unknown" and dropped on inbound; `Some(Tagged(id))`
/// when an id is present.
pub(crate) fn parse_agent_marker(body: &str) -> Option<AgentMarker> {
    let trimmed = body.trim_start();
    let rest = trimmed.strip_prefix(MARKER_PREFIX)?;
    // `rest` now begins with either ` -->` (bare) or `:<id> -->` (tagged).
    if let Some(after_colon) = rest.strip_prefix(':') {
        if let Some(end) = after_colon.find("-->") {
            let id = after_colon[..end].trim();
            if id.is_empty() {
                return Some(AgentMarker::Bare);
            }
            return Some(AgentMarker::Tagged(id.to_string()));
        }
        return Some(AgentMarker::Bare);
    }
    if rest.trim_start().starts_with("-->") {
        return Some(AgentMarker::Bare);
    }
    None
}

/// Inputs for [`pr_opened_text`] — grouped to keep the signature small.
struct PrOpenedText<'a> {
    pr_number: u64,
    title: &'a str,
    sender: &'a str,
    head_ref: &'a str,
    base_ref: &'a str,
    html_url: &'a str,
    owner: &'a str,
    repo: &'a str,
}

/// Initial task message for a newly opened PR. Deliberately minimal — the PR
/// description and comments evolve after task creation, so instead of
/// inlining them (they'd go stale) the agent is pointed at the `moat gh`
/// commands that read the live state.
fn pr_opened_text(p: &PrOpenedText<'_>) -> String {
    let mut text = format!(
        "## New Pull Request: #{}\n\n\
         **Title:** {}\n\
         **Author:** @{}\n\
         **Branch:** {} → {}\n",
        p.pr_number, p.title, p.sender, p.head_ref, p.base_ref,
    );

    if !p.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", p.html_url);
    }

    let (pr_number, owner, repo) = (p.pr_number, p.owner, p.repo);
    let _ = write!(
        text,
        "\nBefore working on this PR, fetch the current state — the description \
         and comments evolve and must be read live, not from this message:\n\n\
         - Overview and description: `moat gh pr view {pr_number} --comments`\n\
         - Unified diff: `moat gh pr diff {pr_number}`\n\
         - Review/inline comments (with ids): `moat gh api repos/{owner}/{repo}/pulls/{pr_number}/comments`\n\
         - Conversation comments (with ids): `moat gh api repos/{owner}/{repo}/issues/{pr_number}/comments`\n"
    );

    text
}

/// Task message for an `issue_comment` webhook. Carries the PR/issue number
/// and the numeric comment id so the agent can reply via
/// `moat gh pr reply` / the GitHub API without guessing ids.
fn issue_comment_text(sender: &str, number: u64, comment_id: u64, body: &str) -> String {
    format!("**@{sender}** commented on #{number} (comment id: {comment_id}):\n\n{body}")
}

/// Inputs for [`review_comment_text`] — grouped to keep the signature small.
struct ReviewCommentText<'a> {
    sender: &'a str,
    path: &'a str,
    start_line: Option<u64>,
    line: Option<u64>,
    html_url: &'a str,
    comment_id: u64,
    in_reply_to_id: Option<u64>,
    diff_hunk: &'a str,
    body: &'a str,
}

/// Task message for a `pull_request_review_comment` webhook (inline comment).
fn review_comment_text(p: &ReviewCommentText<'_>) -> String {
    let mut text = format!("**@{}** commented on `{}`", p.sender, p.path);

    match (p.start_line, p.line) {
        (Some(start), Some(end)) if start != end => {
            let _ = write!(text, " (lines {start}-{end})");
        }
        (_, Some(ln)) => {
            let _ = write!(text, " (line {ln})");
        }
        _ => {}
    }
    text.push_str(":\n");

    if !p.html_url.is_empty() {
        let _ = writeln!(text, "{}", p.html_url);
    }
    let _ = writeln!(text, "Comment id: {}", p.comment_id);

    if let Some(reply_id) = p.in_reply_to_id {
        let _ = write!(text, "\n> In reply to comment {reply_id}\n");
    }

    if !p.diff_hunk.is_empty() {
        let _ = write!(text, "\n```diff\n{}\n```\n", p.diff_hunk);
    }

    let _ = write!(text, "\n{}", p.body);
    text
}

/// Task message for a submitted `pull_request_review`. The review's inline
/// comments arrive as separate review-comment events; the trailing hint tells
/// the agent how to list them together by review id.
fn review_text(
    sender: &str,
    state_label: &str,
    review_id: u64,
    body: &str,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> String {
    format!(
        "**@{sender}** — {state_label} (review id: {review_id}):\n\n{body}\n\n\
         Inline comments on this review: \
         `moat gh api repos/{owner}/{repo}/pulls/{pr_number}/reviews/{review_id}/comments`"
    )
}

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

/// Outcome of `resolve_and_process_github_webhook`, surfaced so the HTTP
/// handler can emit accurate metrics per outcome / reason / event_type.
pub enum GithubWebhookOutcome {
    Processed { event_type: String },
    Ignored {
        event_type: String,
        reason: &'static str,
    },
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
    /// Database pool — currently unused by token resolution (handled by the
    /// canonical resolver via `provider_tokens`), kept for future per-pool DB
    /// needs and to remain wire-compatible with the existing public ctor.
    #[allow(dead_code)] // reason: held for future per-adapter DB needs
    db: DbPool,
    /// Provider-token repository — consulted by the canonical GitHub-token
    /// resolver to read the user's `provider_tokens` row.
    provider_tokens: Arc<dyn ProviderTokenRepository>,
    /// Provider-token auth repository — needed to persist refreshed GitHub App
    /// user tokens. `None` is acceptable in tests / minimal wiring.
    provider_token_auth: Option<Arc<dyn ProviderTokenAuthRepository>>,
    /// Default GitHub App service handle — used by the resolver to refresh
    /// expiring `github_app` user tokens. `None` when no default App is
    /// configured.
    github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps — consulted by the resolver to
    /// mint installation tokens for bot users (`auth_method =
    /// 'github_app_installation'`).
    github_apps: Arc<GitHubAppRegistry>,
    /// Deduplication cache: delivery_id → () with 5-minute TTL.
    dedup_cache: Cache<String, ()>,
}

impl GithubPrAdapter {
    pub fn new(
        global_webhook_secret: Option<String>,
        global_bot_token: Option<String>,
        db: DbPool,
        provider_tokens: Arc<dyn ProviderTokenRepository>,
        provider_token_auth: Option<Arc<dyn ProviderTokenAuthRepository>>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        Self {
            global_webhook_secret,
            global_bot_token,
            db,
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            dedup_cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Resolve a GitHub token for API calls keyed by user id and repo owner.
    ///
    /// Dispatches through the canonical resolver so bot users
    /// (`auth_method = 'github_app_installation'`) mint a fresh installation
    /// token instead of returning the sentinel placeholder. Order:
    ///
    /// 1. `provider_tokens(user_id)` via the canonical resolver — branches on
    ///    `auth_method` and mints / refreshes as needed. Empty tokens are
    ///    treated as missing.
    /// 2. Global `GITHUB_CHANNEL_TOKEN` PAT fallback.
    ///
    /// `repo_owner` is required to mint installation tokens for bot users
    /// because installations are scoped per-org.
    pub(crate) async fn resolve_token_for_user(
        &self,
        user_id: &str,
        repo_owner: Option<&str>,
    ) -> Result<String, AdapterError> {
        match resolve_user_github_token(
            self.provider_tokens.as_ref(),
            self.provider_token_auth.as_deref(),
            self.github_app_service.as_deref(),
            self.github_apps.as_ref(),
            user_id,
            repo_owner,
        )
        .await
        {
            Ok(Some(token)) if !token.is_empty() => return Ok(token),
            Ok(_) => {}
            Err(e) => {
                return Err(AdapterError::AdapterApi(format!(
                    "Failed to resolve GitHub token for user {user_id}: {e}"
                )));
            }
        }

        // Empty global tokens are treated as missing (e.g. compose files
        // passing `GITHUB_CHANNEL_TOKEN: ${GITHUB_CHANNEL_TOKEN:-}`).
        if let Some(ref token) = self.global_bot_token {
            if !token.is_empty() {
                return Ok(token.clone());
            }
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

    /// Resolve a GitHub token for reactions.
    ///
    /// Priority:
    /// 1. The binding's `filter_user_id` (the task owner) via the canonical
    ///    resolver — `repo_owner` is required so bot users get a per-org
    ///    installation token.
    /// 2. The global `GITHUB_CHANNEL_TOKEN` PAT fallback.
    async fn resolve_reaction_token(
        &self,
        binding: &BindingContext,
        repo_owner: Option<&str>,
    ) -> Result<String, AdapterError> {
        let user_id = binding
            .destination_config
            .get("filter_user_id")
            .or_else(|| binding.metadata.get("filter_user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !user_id.is_empty() {
            return self.resolve_token_for_user(user_id, repo_owner).await;
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

        // Drop events from ignored senders (`linear-code[bot]` by default;
        // override per trigger via `provider_config.ignored_senders`). Applies
        // to ALL event types, including `pull_request` opened/closed — ignored
        // means ignored.
        if !sender_login.is_empty()
            && rule_config
                .ignored_senders
                .iter()
                .any(|s| s.eq_ignore_ascii_case(sender_login))
        {
            debug!(
                sender_login,
                event_type, "GitHub event dropped: sender in ignored_senders"
            );
            return Ok(GithubPrWebhookResult::Ignore);
        }

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

        let text = pr_opened_text(&PrOpenedText {
            pr_number,
            title,
            sender: ctx.sender_login,
            head_ref,
            base_ref,
            html_url,
            owner: ctx.owner,
            repo: ctx.repo,
        });

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
            source_task_id: None,
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
            source_task_id: None,
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

        let source_task_id = match parse_agent_marker(comment_body) {
            Some(AgentMarker::Bare) => {
                debug!(
                    comment_id,
                    "Ignoring self-authored agent comment (legacy bare marker)"
                );
                return Ok(GithubPrWebhookResult::Ignore);
            }
            Some(AgentMarker::Tagged(task_id)) => Some(task_id),
            None => None,
        };

        let user_id = self
            .resolve_user_or_fallback(ctx.sender_login, ctx.rule, ctx.db)
            .await;

        Ok(GithubPrWebhookResult::Message(Box::new(InboundMessage {
            provider: AdapterKind::GithubPr,
            source_id: ctx.rule.source_matcher.clone(),
            thread_id: Some(pr_number.to_string()),
            message_thread_id: pr_number.to_string(),
            user_id,
            text: issue_comment_text(ctx.sender_login, pr_number, comment_id, comment_body),
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": ctx.owner,
                "repo": ctx.repo,
                "pr_number": pr_number,
                "comment_id": comment_id,
            }),
            provider_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::issue_comment(comment_id)),
            source_task_id,
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

        let source_task_id = match parse_agent_marker(comment_body) {
            Some(AgentMarker::Bare) => {
                debug!(
                    comment_id,
                    "Ignoring self-authored agent review comment (legacy bare marker)"
                );
                return Ok(GithubPrWebhookResult::Ignore);
            }
            Some(AgentMarker::Tagged(task_id)) => Some(task_id),
            None => None,
        };

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

        let text = review_comment_text(&ReviewCommentText {
            sender: ctx.sender_login,
            path,
            start_line,
            line,
            html_url,
            comment_id,
            in_reply_to_id,
            diff_hunk,
            body: comment_body,
        });

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
            source_task_id,
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

        let text = review_text(
            ctx.sender_login,
            state_label,
            review_id,
            review_body,
            ctx.owner,
            ctx.repo,
            pr_number,
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
            source_task_id: None,
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
            source_task_id: None,
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
            source_task_id: None,
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

    fn reply_instructions(&self) -> Option<String> {
        Some(
            "## GitHub PR Agent Notes\n\
             - Always read the live PR state via the `moat gh` commands from the \
               initial message (`pr view`, `pr diff`, `api …/comments`) — the \
               description and comments evolve after task creation.\n\
             - `moat gh pr reply <pr_number> <comment_id> --body \"…\"` — post a \
               threaded reply to a specific review/inline comment by its numeric \
               id (shown as `comment id:` in delivered messages and in \
               `moat gh api …/pulls/<pr>/comments` output).\n\
             - `moat gh pr comment <pr_number> --body \"…\"` — post a top-level \
               conversation comment.\n\
             - `moat messages reply --message <message_id> --text \"…\"` — reply \
               to a delivered message; routes as a threaded reply when the \
               message came from an inline review comment, top-level otherwise.\n\
             - Task completion does NOT auto-post anything to GitHub — when you \
               finish, post your final summary explicitly via \
               `moat gh pr comment`.\n\
             - Markdown is supported in all comment bodies."
                .to_string(),
        )
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
        let token = self.resolve_token_for_user(&user_id, Some(&owner)).await?;
        let api = GithubPrApi::new(&token);

        let mut body = format!("{}\n{}", agent_marker(&binding.task_id), message.text);
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
                let token = self.resolve_token_for_user(&user_id, Some(&owner)).await?;
                let api = GithubPrApi::new(&token);

                let mut body = format!("{}\n{}", agent_marker(&binding.task_id), message.text);
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
        let token = self.resolve_reaction_token(binding, Some(&owner)).await?;
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
        let token = self.resolve_reaction_token(binding, Some(&owner)).await?;
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
        let token = match self.resolve_token_for_user(user_id, Some(&owner)).await {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let api = GithubPrApi::new(&token);

        let comments = api.list_comments(&owner, &repo, pr_number, limit).await?;

        if comments.is_empty() {
            return Ok(None);
        }

        let ignored_senders = super::config::ignored_senders(config.provider_config.as_ref());
        Ok(pr_comments_context(
            &comments,
            &thread.task_id,
            exclude_external_ids,
            &ignored_senders,
        ))
    }
}

/// Build the "## PR Comments" context block from fetched PR comments.
/// Mirrors the webhook ingress filters: drops this task's own agent comments,
/// already-delivered comment ids, and ignored senders (so e.g.
/// `linear-code[bot]` linkback noise doesn't leak in via context backfill
/// even though it never arrives as a webhook delivery).
fn pr_comments_context(
    comments: &[super::api::PrComment],
    task_id: &str,
    exclude_external_ids: &[String],
    ignored_senders: &[String],
) -> Option<String> {
    let context = comments
        .iter()
        .filter(|c| match c.body.as_deref().and_then(parse_agent_marker) {
            // Hide this task's own outbound comments — they're already in
            // its message log and would only echo back as context.
            Some(AgentMarker::Tagged(ref tid)) => tid != task_id,
            // Legacy bare markers: source task unknown, drop to match
            // pre-tagged behavior.
            Some(AgentMarker::Bare) => false,
            None => true,
        })
        .filter(|c| {
            let prefixed = super::external_id::issue_comment(c.id);
            let bare = c.id.to_string();
            !exclude_external_ids.contains(&prefixed) && !exclude_external_ids.contains(&bare)
        })
        .filter(|c| {
            !ignored_senders
                .iter()
                .any(|s| s.eq_ignore_ascii_case(c.author()))
        })
        .map(|c| format!("@{}: {}", c.author(), c.body.as_deref().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n\n");

    if context.is_empty() {
        None
    } else {
        Some(format!("## PR Comments\n\n{context}"))
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
) -> Result<GithubWebhookOutcome, AdapterError> {
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

    debug!(
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
        debug!(
            repo = repo_full_name,
            owner = owner_login,
            "GitHub webhook ignored: no provider_connection / task_trigger"
        );
        return Ok(GithubWebhookOutcome::Ignored {
            event_type: event_type.to_string(),
            reason: "no_trigger",
        });
    };

    if !rule.active {
        debug!(
            repo = repo_full_name,
            "GitHub webhook ignored: task trigger inactive"
        );
        return Ok(GithubWebhookOutcome::Ignored {
            event_type: event_type.to_string(),
            reason: "inactive",
        });
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

    Ok(GithubWebhookOutcome::Processed {
        event_type: event_type.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::Adapter;

    #[tokio::test]
    async fn provider_keys_separate_channel_and_integration_storage() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        assert_eq!(provider.provider_key(), "github_pr");
        assert_eq!(provider.connection_provider_key(), "github");
        assert!(!provider.allow_default_connection_fallback());
        assert!(!provider.allow_wildcard_trigger_fallback());
    }

    #[tokio::test]
    async fn binding_destination_from_existing_thread_builds_pr_subject() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
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
    fn pr_opened_text_is_minimal_with_live_read_instructions() {
        let text = pr_opened_text(&PrOpenedText {
            pr_number: 42,
            title: "feat: x",
            sender: "alice",
            head_ref: "feat/x",
            base_ref: "main",
            html_url: "https://github.com/o/r/pull/42",
            owner: "o",
            repo: "r",
        });
        assert!(text.contains("## New Pull Request: #42"));
        assert!(text.contains("**Title:** feat: x"));
        assert!(text.contains("**Author:** @alice"));
        assert!(text.contains("**Branch:** feat/x → main"));
        assert!(text.contains("**Link:** https://github.com/o/r/pull/42"));
        // The stale-prone description is no longer inlined…
        assert!(!text.contains("### Description"));
        // …replaced by live-read CLI instructions.
        assert!(text.contains("`moat gh pr view 42 --comments`"));
        assert!(text.contains("`moat gh pr diff 42`"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42/comments`"));
        assert!(text.contains("`moat gh api repos/o/r/issues/42/comments`"));
    }

    #[test]
    fn pr_opened_text_omits_link_line_when_url_missing() {
        let text = pr_opened_text(&PrOpenedText {
            pr_number: 7,
            title: "t",
            sender: "bob",
            head_ref: "h",
            base_ref: "b",
            html_url: "",
            owner: "o",
            repo: "r",
        });
        assert!(!text.contains("**Link:**"));
    }

    #[test]
    fn issue_comment_text_carries_number_and_comment_id() {
        let text = issue_comment_text("alice", 42, 123_456, "looks good");
        assert_eq!(
            text,
            "**@alice** commented on #42 (comment id: 123456):\n\nlooks good"
        );
    }

    #[test]
    fn review_comment_text_carries_comment_id() {
        let text = review_comment_text(&ReviewCommentText {
            sender: "alice",
            path: "src/lib.rs",
            start_line: None,
            line: Some(10),
            html_url: "https://github.com/o/r/pull/42#discussion_r777",
            comment_id: 777,
            in_reply_to_id: None,
            diff_hunk: "+ let x = 1;",
            body: "rename this",
        });
        assert!(text.contains("**@alice** commented on `src/lib.rs` (line 10):"));
        assert!(text.contains("https://github.com/o/r/pull/42#discussion_r777"));
        assert!(text.contains("Comment id: 777"));
        assert!(text.contains("```diff\n+ let x = 1;\n```"));
        assert!(text.ends_with("rename this"));
    }

    #[test]
    fn review_text_carries_review_id_and_inline_comments_hint() {
        let text = review_text("alice", "Changes Requested", 555, "see notes", "o", "r", 42);
        assert!(text.starts_with("**@alice** — Changes Requested (review id: 555):\n\nsee notes"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42/reviews/555/comments`"));
    }

    #[tokio::test]
    async fn reply_instructions_cover_reply_paths_and_completion_behavior() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let notes = provider.reply_instructions().expect("instructions");
        assert!(notes.contains("## GitHub PR Agent Notes"));
        assert!(notes.contains("moat gh pr reply <pr_number> <comment_id>"));
        assert!(notes.contains("moat gh pr comment <pr_number>"));
        assert!(notes.contains("moat messages reply --message"));
        assert!(notes.contains("does NOT auto-post"));
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
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
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

    fn fake_provider_tokens() -> Arc<dyn ProviderTokenRepository> {
        Arc::new(crate::auth::provider_token_dao::PgProviderTokenRepository(
            fake_db_pool(),
        ))
    }

    fn fake_github_apps() -> Arc<GitHubAppRegistry> {
        Arc::new(GitHubAppRegistry::default())
    }

    fn fake_rule() -> TaskTriggerRoute {
        use crate::adapters::models::RoutingMode;
        use crate::scope::Scope;
        use chrono::Utc;
        TaskTriggerRoute {
            id: "ir_test".into(),
            provider: "github".into(),
            connection_id: "pc_test".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            run_as_user_id: None,
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            allow_direct_send: false,
            provider_config: None,
            ongoing_task_id: None,
            close_task_on_close: false,
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

    #[test]
    fn parse_agent_marker_classifies_bodies() {
        // No marker.
        assert!(parse_agent_marker("hello world").is_none());
        assert!(parse_agent_marker("").is_none());

        // Legacy bare marker (still emitted by old deployments).
        assert_eq!(
            parse_agent_marker("<!-- moatless-agent -->\nbody"),
            Some(AgentMarker::Bare)
        );

        // Leading whitespace is allowed.
        assert_eq!(
            parse_agent_marker("   \n<!-- moatless-agent -->\nbody"),
            Some(AgentMarker::Bare)
        );

        // Tagged.
        assert_eq!(
            parse_agent_marker("<!-- moatless-agent:task-abc -->\nbody"),
            Some(AgentMarker::Tagged("task-abc".to_string()))
        );

        // Tagged with surrounding whitespace around the id (defensive trim).
        assert_eq!(
            parse_agent_marker("<!-- moatless-agent: task-abc  -->\nbody"),
            Some(AgentMarker::Tagged("task-abc".to_string()))
        );

        // Malformed: empty id falls back to Bare so we still drop legacy-style.
        assert_eq!(
            parse_agent_marker("<!-- moatless-agent: -->\nbody"),
            Some(AgentMarker::Bare)
        );

        // Marker that does not actually begin at start-after-trim is not matched.
        assert!(parse_agent_marker("real comment\n<!-- moatless-agent:x -->").is_none());
    }

    #[test]
    fn agent_marker_round_trips_through_parser() {
        let body = format!("{}\nhello", agent_marker("task-42"));
        assert_eq!(
            parse_agent_marker(&body),
            Some(AgentMarker::Tagged("task-42".to_string()))
        );
    }

    #[tokio::test]
    async fn issue_comment_with_legacy_bare_marker_is_ignored() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let rule = fake_rule();
        let db = fake_db_pool();
        let ctx = EventContext {
            rule: &rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "SoapLabBot",
            db: &db,
        };
        let payload = serde_json::json!({
            "action": "created",
            "issue": {
                "number": 42,
                "pull_request": { "url": "https://api.github.com/repos/soaplabs/repo/pulls/42" }
            },
            "comment": {
                "id": 123_456_u64,
                "body": "<!-- moatless-agent -->\nAcknowledged. Waiting for a fix commit.",
            }
        });

        let result = provider
            .handle_issue_comment_event(&payload, &ctx)
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Ignore));
    }

    /// Full issue_comment payload from a given sender, suitable for
    /// `parse_webhook` (includes `repository.full_name` and `sender.login`).
    fn issue_comment_payload(sender: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "created",
            "repository": { "full_name": "soaplabs/repo" },
            "sender": { "login": sender },
            "issue": {
                "number": 42,
                "pull_request": { "url": "https://api.github.com/repos/soaplabs/repo/pulls/42" }
            },
            "comment": {
                "id": 123_456_u64,
                "body": "<!-- linear-linkback -->\nSP-1529 linked",
            }
        })
    }

    #[tokio::test]
    async fn ignored_sender_comment_is_dropped_at_ingress() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let mut rule = fake_rule();
        // Mixed case in config vs payload — match must be case-insensitive.
        rule.provider_config =
            Some(serde_json::json!({"ignored_senders": ["Linear-Code[bot]"]}));
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
                "delivery-ignored-sender",
                "issue_comment",
                &issue_comment_payload("linear-code[bot]"),
                &rule,
                &db,
            )
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Ignore));
    }

    fn pr_comment(id: u64, login: &str, body: &str) -> crate::adapters::github_pr::api::PrComment {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "user": {"login": login},
            "body": body,
            "created_at": "2026-06-12T00:00:00Z",
        }))
        .expect("valid PrComment json")
    }

    #[test]
    fn context_backfill_drops_default_ignored_senders() {
        let comments = vec![
            pr_comment(1, "linear-code[bot]", "<!-- linear-linkback -->\nSP-1529 linked"),
            pr_comment(2, "alice", "looks good"),
        ];
        let ignored = crate::adapters::github_pr::config::ignored_senders(None);

        let context = pr_comments_context(&comments, "task-1", &[], &ignored).unwrap();

        assert!(context.contains("@alice: looks good"));
        assert!(!context.contains("linear-code[bot]"));
    }

    #[test]
    fn context_backfill_keeps_ignored_sender_when_opted_out() {
        let comments = vec![pr_comment(
            1,
            "linear-code[bot]",
            "<!-- linear-linkback -->\nSP-1529 linked",
        )];

        let context = pr_comments_context(&comments, "task-1", &[], &[]).unwrap();

        assert!(context.contains("@linear-code[bot]"));
    }

    #[test]
    fn context_backfill_returns_none_when_all_comments_ignored() {
        let comments = vec![pr_comment(
            1,
            "linear-code[bot]",
            "<!-- linear-linkback -->\nSP-1529 linked",
        )];
        let ignored = crate::adapters::github_pr::config::ignored_senders(None);

        assert!(pr_comments_context(&comments, "task-1", &[], &ignored).is_none());
    }

    #[tokio::test]
    async fn linear_code_bot_dropped_by_default() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        // No provider_config at all — the built-in default ignore list applies.
        let rule = fake_rule();
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
                "delivery-default-ignored-sender",
                "issue_comment",
                &issue_comment_payload("linear-code[bot]"),
                &rule,
                &db,
            )
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Ignore));
    }

    #[tokio::test]
    async fn empty_ignored_senders_opt_out_delivers() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        // Explicit empty list opts out of the default — the same bot comment
        // goes through.
        let mut rule = fake_rule();
        rule.provider_config = Some(serde_json::json!({"ignored_senders": []}));
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
                "delivery-opt-out-sender",
                "issue_comment",
                &issue_comment_payload("linear-code[bot]"),
                &rule,
                &db,
            )
            .await
            .unwrap();

        match result {
            GithubPrWebhookResult::Message(msg) => {
                assert!(msg.text.contains("@linear-code[bot]"));
            }
            _ => panic!("expected Message for opted-out sender filter"),
        }
    }

    #[tokio::test]
    async fn issue_comment_with_tagged_marker_delivers_with_source_task_id() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let rule = fake_rule();
        let db = fake_db_pool();
        let ctx = EventContext {
            rule: &rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "SoapLabBot",
            db: &db,
        };
        let payload = serde_json::json!({
            "action": "created",
            "issue": {
                "number": 42,
                "pull_request": { "url": "https://api.github.com/repos/soaplabs/repo/pulls/42" }
            },
            "comment": {
                "id": 123_456_u64,
                "body": format!("{}\nAcknowledged. Waiting for a fix commit.", agent_marker("task-A")),
            }
        });

        let result = provider
            .handle_issue_comment_event(&payload, &ctx)
            .await
            .unwrap();

        let msg = match result {
            GithubPrWebhookResult::Message(m) => m,
            _ => panic!("expected Message"),
        };
        assert_eq!(msg.source_task_id.as_deref(), Some("task-A"));
    }

    #[tokio::test]
    async fn issue_comment_from_human_is_delivered() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let rule = fake_rule();
        let db = fake_db_pool();
        let ctx = EventContext {
            rule: &rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "alice",
            db: &db,
        };
        let payload = serde_json::json!({
            "action": "created",
            "issue": {
                "number": 42,
                "pull_request": { "url": "https://api.github.com/repos/soaplabs/repo/pulls/42" }
            },
            "comment": {
                "id": 123_456_u64,
                "body": "Please re-check line 42 after the fix commit.",
            }
        });

        let result = provider
            .handle_issue_comment_event(&payload, &ctx)
            .await
            .unwrap();

        let msg = match result {
            GithubPrWebhookResult::Message(m) => m,
            _ => panic!("expected Message"),
        };
        assert_eq!(msg.source_task_id, None);
    }

    #[tokio::test]
    async fn review_comment_with_legacy_bare_marker_is_ignored() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let rule = fake_rule();
        let db = fake_db_pool();
        let ctx = EventContext {
            rule: &rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "SoapLabBot",
            db: &db,
        };
        let payload = serde_json::json!({
            "action": "created",
            "pull_request": { "number": 42 },
            "comment": {
                "id": 999_u64,
                "body": "   \n<!-- moatless-agent -->\nAcknowledged on the inline thread.",
                "path": "src/foo.rs",
                "diff_hunk": "@@ -1 +1 @@",
                "html_url": "https://github.com/soaplabs/repo/pull/42#discussion_r999",
            }
        });

        let result = provider
            .handle_review_comment_event(&payload, &ctx)
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Ignore));
    }

    #[tokio::test]
    async fn review_comment_with_tagged_marker_delivers_with_source_task_id() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let rule = fake_rule();
        let db = fake_db_pool();
        let ctx = EventContext {
            rule: &rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: "SoapLabBot",
            db: &db,
        };
        let payload = serde_json::json!({
            "action": "created",
            "pull_request": { "number": 42 },
            "comment": {
                "id": 999_u64,
                "body": format!("{}\nAcknowledged on the inline thread.", agent_marker("task-B")),
                "path": "src/foo.rs",
                "diff_hunk": "@@ -1 +1 @@",
                "html_url": "https://github.com/soaplabs/repo/pull/42#discussion_r999",
            }
        });

        let result = provider
            .handle_review_comment_event(&payload, &ctx)
            .await
            .unwrap();

        let msg = match result {
            GithubPrWebhookResult::Message(m) => m,
            _ => panic!("expected Message"),
        };
        assert_eq!(msg.source_task_id.as_deref(), Some("task-B"));
    }
}

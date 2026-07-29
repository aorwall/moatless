//! GitHub PR adapter — Adapter trait impl + webhook parsing.

use std::fmt::Write;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, Audience, BindingContext, ExternalStatus, InboundMessage, LoopContext,
    OutboundReply, SubscriptionMatch, TaskBinding,
};
use crate::adapters::{Adapter, AdapterKind, BindingDestination};
use crate::auth::credentials::resolve_user_github_token;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::db::DbPool;
use crate::secret::service::SecretService;
use crate::state::AppState;

use super::api::GithubPrApi;
use super::config::GithubPrRuleConfig;
use super::signals::{MergeableState, ReviewDecision};
use super::status::{self, github_external_status};

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

/// Whether a PR title matches any of the configured ignore regexes. Patterns
/// are compiled case-insensitively; an invalid regex is logged and treated as
/// non-matching so one bad user-supplied pattern can't drop every event.
fn title_is_ignored(title: &str, patterns: &[String]) -> bool {
    patterns.iter().any(
        |p| match regex::RegexBuilder::new(p).case_insensitive(true).build() {
            Ok(re) => re.is_match(title),
            Err(e) => {
                warn!(pattern = %p, error = %e, "Invalid ignored_title_patterns regex; skipping");
                false
            }
        },
    )
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
    // A bare verdict (e.g. an approval submitted with no summary) has an empty
    // body — render a placeholder so the message reads cleanly instead of
    // leaving a blank gap.
    let body_section = if body.is_empty() {
        "_(no review comment)_"
    } else {
        body
    };
    format!(
        "**@{sender}** — {state_label} (review id: {review_id}):\n\n{body_section}\n\n\
         Inline comments on this review: \
         `moat gh api repos/{owner}/{repo}/pulls/{pr_number}/reviews/{review_id}/comments`"
    )
}

/// Task message for a PR that GitHub reports as having merge conflicts
/// (`mergeable_state == "dirty"`). Points the agent at the live diff and
/// suggests the usual resolution path.
fn merge_conflict_text(owner: &str, repo: &str, pr_number: u64, html_url: &str) -> String {
    let mut text = format!(
        "## ⚠️ Merge Conflicts on PR #{pr_number}\n\n\
         GitHub reports this PR can no longer be merged cleanly — the head \
         branch has conflicts with the base branch.\n",
    );

    if !html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {html_url}");
    }

    let _ = write!(
        text,
        "\nInspect and resolve the conflicts, then push the fix:\n\n\
         - Current diff: `moat gh pr diff {pr_number}`\n\
         - PR overview: `moat gh pr view {pr_number}`\n\
         - Conflicting files: `moat gh api repos/{owner}/{repo}/pulls/{pr_number}` (see `mergeable_state`)\n"
    );

    text
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
#[derive(Debug)]
pub enum GithubWebhookOutcome {
    Processed {
        event_type: String,
    },
    Ignored {
        event_type: String,
        reason: &'static str,
    },
}

/// Metadata about a closed PR, captured at parse time and used downstream
/// to compose the auto-close chat notice.
///
/// Serializable because it is carried as the payload of a `github.close`
/// Adapter delivery record (see [`super::delivery`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrCloseInfo {
    pub pr_number: u64,
    pub merged: bool,
    pub title: String,
    pub html_url: String,
    pub sender_login: String,
}

/// Context passed through event handlers to avoid too many function arguments.
struct EventContext<'a> {
    rule: &'a SubscriptionMatch,
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
    /// Frontend base URL (`config.auth.frontend_url`). Used to render the
    /// `Moatless task:` footer link on outbound comments and to recover the
    /// authoring task id from that link on inbound echoes / context backfill.
    frontend_url: String,
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
    /// Secret service — resolves the encrypted PAT behind a `secret_ref`
    /// provider-token row. `None` in tests / minimal wiring.
    secrets: Option<Arc<SecretService>>,
}

impl GithubPrAdapter {
    // reason: wiring ctor — each arg is a distinct injected dependency
    // (secrets, frontend URL, db pool, token repos, App registry) with no
    // natural grouping; a params struct would only add indirection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        global_webhook_secret: Option<String>,
        global_bot_token: Option<String>,
        frontend_url: String,
        db: DbPool,
        provider_tokens: Arc<dyn ProviderTokenRepository>,
        provider_token_auth: Option<Arc<dyn ProviderTokenAuthRepository>>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        Self {
            global_webhook_secret,
            global_bot_token,
            frontend_url,
            db,
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets: None,
        }
    }

    /// Wire the secret service used to resolve `secret_ref` provider tokens.
    #[must_use]
    pub fn with_secrets(mut self, secrets: Arc<SecretService>) -> Self {
        self.secrets = Some(secrets);
        self
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
            self.secrets.as_deref(),
            user_id,
            repo_owner,
        )
        .await
        {
            Ok(Some(token)) if !token.is_empty() => return Ok(token),
            Ok(_) => {}
            Err(source) => {
                return Err(AdapterError::UserCredential {
                    user_id: user_id.to_string(),
                    source,
                });
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

    /// Resolve the system user whose GitHub credentials should author an outbound
    /// reply. Prefers the owning task's `author_user_id` (passed from the task's
    /// `owner_user_id`), then the binding's connect-time `filter_user_id`, then the
    /// trigger's run-as user. A human-owned task connected to a PR governed by a
    /// separate review trigger thus replies under the human's identity rather than
    /// the bot's; a trigger-created automation task (no owner/filter) still replies
    /// under its `runAsUserId`.
    fn outbound_user_id(
        author_user_id: Option<&str>,
        binding: &TaskBinding,
        rule: Option<&SubscriptionMatch>,
    ) -> Option<String> {
        author_user_id
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                binding
                    .destination_config
                    .get("filter_user_id")
                    .or_else(|| binding.metadata.get("filter_user_id"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                rule.map(|r| r.effective_user_id().to_string())
                    .filter(|s| !s.is_empty())
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
    ///
    /// Pure with respect to the delivery: it decides *what* the event means and
    /// never decides whether the delivery has been seen before. Collapsing a
    /// redelivery is durable admission's job — see
    /// [`crate::adapters::github_pr::delivery`].
    pub async fn parse_webhook(
        &self,
        event_type: &str,
        payload: &serde_json::Value,
        rule: &SubscriptionMatch,
        db: &DbPool,
    ) -> Result<GithubPrWebhookResult, AdapterError> {
        let rule_config = GithubPrRuleConfig::from_rule(rule);

        // Filter by events_filter config
        if rule_config.event_filtered_out(event_type) {
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
        if rule_config.is_ignored_sender(sender_login) {
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

    /// Status-relevant `pull_request` actions — those that can change a PR's
    /// displayed state. Other actions (`labeled`, `assigned`, …) leave status
    /// untouched.
    const STATUS_ACTIONS: &'static [&'static str] = &[
        "opened",
        "reopened",
        "closed",
        "ready_for_review",
        "converted_to_draft",
        "edited",
    ];

    /// Persist the PR's external status onto every binding for its subject.
    ///
    /// Derives `open`/`merged`/`closed`/`draft` from
    /// `pull_request.{state,merged,draft}` and writes it to the dedicated
    /// `external_status*` columns (which `upsert` does not touch). The DB work
    /// is spawned so webhook parsing/ack is never blocked on it; best-effort,
    /// failures are logged.
    fn persist_pr_status(&self, action: &str, payload: &serde_json::Value, ctx: &EventContext<'_>) {
        if !Self::STATUS_ACTIONS.contains(&action) {
            return;
        }
        let Some(pr) = payload.get("pull_request") else {
            return;
        };
        let Some(pr_number) = pr.get("number").and_then(serde_json::Value::as_u64) else {
            return;
        };
        let state = pr
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let merged = pr
            .get("merged")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let draft = pr
            .get("draft")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let title = pr.get("title").and_then(serde_json::Value::as_str);
        let html_url = pr.get("html_url").and_then(serde_json::Value::as_str);

        let status = github_external_status(state, merged, draft, title, html_url);
        // adapter_kind="github", subject_kind="github_pr" — matches
        // runtime.rs ADAPTER / SUBJECT_KIND.
        let subject_key = format!("{}/{}#{pr_number}", ctx.owner, ctx.repo);
        let db = self.db.clone();

        tokio::spawn(async move {
            let repo = PgTaskBindingRepository(db);
            let bindings = match repo
                .find_by_subject("github", "github_pr", &subject_key)
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    warn!(
                        subject_key = %subject_key,
                        error = %e,
                        "Failed to look up bindings for PR status persistence"
                    );
                    return;
                }
            };
            for binding in bindings {
                if let Err(e) = repo
                    .update_external_status(
                        &binding.id,
                        status.status.as_deref(),
                        status.state_kind.as_deref(),
                        &status.raw,
                    )
                    .await
                {
                    warn!(
                        binding_id = %binding.id,
                        error = %e,
                        "Failed to persist PR status onto binding"
                    );
                }
            }
        });
    }

    /// Persist a submitted review's verdict (approved / changes_requested /
    /// dismissed) onto every binding for the PR. Spawned + best-effort, mirroring
    /// [`Self::persist_pr_status`]. Non-verdict states (e.g. a plain `commented`
    /// review) are skipped so they never clobber a real approval.
    fn persist_review_decision(&self, review_state: &str, ctx: &EventContext<'_>, pr_number: u64) {
        let Some(decision) = ReviewDecision::from_review_state(review_state) else {
            return;
        };
        if !decision.is_verdict() {
            return;
        }

        let subject_key = format!("{}/{}#{pr_number}", ctx.owner, ctx.repo);
        let db = self.db.clone();

        tokio::spawn(async move {
            let repo = PgTaskBindingRepository(db);
            let bindings = match repo
                .find_by_subject("github", "github_pr", &subject_key)
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    warn!(
                        subject_key = %subject_key,
                        error = %e,
                        "Failed to look up bindings for review-decision persistence"
                    );
                    return;
                }
            };
            for binding in bindings {
                if let Err(e) = repo
                    .update_review_decision(&binding.id, Some(decision.as_str()))
                    .await
                {
                    warn!(
                        binding_id = %binding.id,
                        error = %e,
                        "Failed to persist review decision onto binding"
                    );
                }
            }
        });
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

        // Persist PR status onto any bindings for this PR, for every
        // status-relevant action — independent of whether the action also
        // spawns or closes a task. Spawned; never blocks parsing.
        self.persist_pr_status(action, payload, ctx);

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

        // Skip dependency/automation PRs whose title matches an ignore regex
        // (e.g. `chore(deps): …`). A match means no review task is created.
        if !rule_config.ignored_title_patterns.is_empty()
            && title_is_ignored(title, &rule_config.ignored_title_patterns)
        {
            info!(
                title,
                "GitHub PR skipped: title matches ignored_title_patterns"
            );
            return Ok(GithubPrWebhookResult::Ignore);
        }

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
            // A pull request opening is the subject itself appearing: every
            // task bound to it hears about it.
            audience: Audience::Everyone,
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
            audience: Audience::Everyone,
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

        // Recover the authoring task id from our `Moatless task:` footer link, if
        // present. Used downstream to drop only the authoring task's own echo.
        let source_task_id =
            crate::adapters::footer::parse_task_id_from_body(&self.frontend_url, comment_body);

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
            // A comment on the pull request itself replies to no thread, so it
            // is everyone's to read.
            audience: Audience::Everyone,
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

        // Recover the authoring task id from our `Moatless task:` footer link, if
        // present. Used downstream to drop only the authoring task's own echo.
        let source_task_id =
            crate::adapters::footer::parse_task_id_from_body(&self.frontend_url, comment_body);

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
            // An inline review comment belongs to its thread. A reply names the
            // comment it answers, and the task that wrote that comment is who
            // it is for — alongside the task whose work is under review. A
            // comment that starts a thread names no parent, so no reviewer is
            // its audience.
            audience: Audience::ParentAuthor {
                parent_external_event_id: in_reply_to_id.map(super::external_id::review_comment),
            },
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

        // Persist the review verdict onto the PR's bindings (best-effort,
        // spawned) independent of whether this review is delivered as a message
        // — so a bare approval is recorded even if its body is empty.
        self.persist_review_decision(review_state, ctx, pr_number);

        // A review carries a verdict (`approved` / `changes_requested`) or is a
        // plain `commented` container. A verdict is meaningful even with no
        // summary text — reviewers very commonly click "Approve" / "Request
        // changes" without typing anything, and dropping those would hide the
        // approval entirely. A `commented` review with an empty body is just an
        // empty wrapper whose inline comments arrive as their own
        // `pull_request_review_comment` events, so it stays ignored.
        let is_verdict = matches!(review_state, "approved" | "changes_requested");
        if review_body.is_empty() && !is_verdict {
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
            audience: Audience::Everyone,
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
            // Whether the pull request builds is its author's concern.
            audience: Audience::SubjectProducer,
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
            audience: Audience::SubjectProducer,
        })))
    }

    /// Resolve a GitHub login to a system user, falling back to the rule's
    /// effective user (run_as_user_id ?? created_by).
    async fn resolve_user_or_fallback(
        &self,
        github_login: &str,
        rule: &SubscriptionMatch,
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
    rule: &SubscriptionMatch,
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

    /// GitHub allows up to 65536 chars per issue/PR comment. Leave headroom for
    /// the optional `Moatless task:` footer that `send_reply_for_binding`
    /// appends.
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
        _config: &LoopContext,
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
        rule: Option<&SubscriptionMatch>,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        let (owner, repo, pr_number) = Self::parse_binding_target(binding)?;
        let user_id = Self::outbound_user_id(message.author_user_id.as_deref(), binding, rule)
            .ok_or_else(|| {
                AdapterError::AdapterApi(
                    "Cannot resolve GitHub token: no task owner, binding filter_user_id, \
                     or task trigger attached"
                        .into(),
                )
            })?;
        let token = self.resolve_token_for_user(&user_id, Some(&owner)).await?;
        let api = GithubPrApi::new(&token);

        // The `Moatless task:` footer (carrying the task URL) doubles as the
        // echo-filter source on the way back in — no hidden marker needed.
        let mut body = message.text.clone();
        if let Some(ref footer) = message.footer {
            body.push_str(footer);
        }

        let comment_id = api.post_comment(&owner, &repo, pr_number, &body).await?;
        Ok(Some(super::external_id::issue_comment(comment_id)))
    }

    async fn reply_to_message(
        &self,
        binding: &TaskBinding,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        target_external_id: &str,
        message: &OutboundReply,
    ) -> Result<Option<String>, AdapterError> {
        // Only PR review comments support threaded replies. Top-level issue
        // comments fall back to the binding-level reply (a new top-level
        // PR comment).
        match super::external_id::comment_target_from_id(target_external_id) {
            Some(super::external_id::CommentTarget::Review(comment_id)) => {
                let (owner, repo, pr_number) = Self::parse_binding_target(binding)?;
                let user_id =
                    Self::outbound_user_id(message.author_user_id.as_deref(), binding, rule)
                        .ok_or_else(|| {
                            AdapterError::AdapterApi(
                                "Cannot resolve GitHub token: no task owner, binding \
                                 filter_user_id, or task trigger attached"
                                    .into(),
                            )
                        })?;
                let token = self.resolve_token_for_user(&user_id, Some(&owner)).await?;
                let api = GithubPrApi::new(&token);

                // The `Moatless task:` footer doubles as the echo-filter source.
                let mut body = message.text.clone();
                if let Some(ref footer) = message.footer {
                    body.push_str(footer);
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

    fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
        status::binding_status_key(binding)
    }

    fn binding_label_and_url(&self, binding: &TaskBinding) -> (String, Option<String>) {
        status::binding_label_and_url(binding)
    }

    fn is_terminal_state_kind(&self, state_kind: &str) -> bool {
        status::is_terminal_state_kind(state_kind)
    }

    async fn fetch_binding_status(
        &self,
        binding: &TaskBinding,
        acting_user: &str,
    ) -> Option<ExternalStatus> {
        let (owner, repo, number) = status::binding_subject(binding)?;

        let token = match self.resolve_token_for_user(acting_user, Some(&owner)).await {
            Ok(token) => token,
            Err(e) => {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to resolve GitHub token for live PR status"
                );
                return None;
            }
        };

        match GithubPrApi::new(&token).get_pr(&owner, &repo, number).await {
            Ok(pr) => Some(status::status_from_pr_info(&pr)),
            Err(e) => {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to fetch live GitHub PR status"
                );
                None
            }
        }
    }
}

/// Process a GitHub webhook: verify signature, parse, and route.
///
/// Called from the HTTP handler.
/// Extract the PR number a webhook is *about*, for the best-effort
/// merge-conflict check that runs on every PR-bearing event. Returns `None` for
/// events with no associated PR (a plain issue comment, a `check_run` not tied
/// to a PR, an unsupported event type) and for `pull_request` `closed` — a
/// closed/merged PR has no meaningful mergeable state to report.
fn merge_conflict_pr_number(event_type: &str, payload: &serde_json::Value) -> Option<u64> {
    let as_u64 = |v: &serde_json::Value| v.as_u64();
    match event_type {
        "pull_request" => {
            let action = payload.get("action").and_then(serde_json::Value::as_str);
            if action == Some("closed") {
                return None;
            }
            payload.pointer("/pull_request/number").and_then(as_u64)
        }
        "pull_request_review" | "pull_request_review_comment" => {
            payload.pointer("/pull_request/number").and_then(as_u64)
        }
        "issue_comment" => {
            // Only PRs have a mergeable state; plain issues don't.
            payload.pointer("/issue/pull_request")?;
            payload.pointer("/issue/number").and_then(as_u64)
        }
        "check_run" => payload
            .pointer("/check_run/pull_requests/0/number")
            .and_then(as_u64),
        "check_suite" => payload
            .pointer("/check_suite/pull_requests/0/number")
            .and_then(as_u64),
        _ => None,
    }
}

/// Merge-conflict check for a PR touched by any webhook event.
///
/// Runs as a `github.merge_conflict` Adapter delivery, so it is off GitHub's
/// response deadline *and* survives a restart between acknowledgment and
/// execution. Polls the PR's mergeable state (GitHub computes it
/// asynchronously) and, if the PR is open and conflicted
/// (`mergeable_state == "dirty"`), delivers a one-time (per head sha, via the
/// event-receipt dedup in [`super::runtime::route_inbound_message`])
/// notification to every task bound to the PR. Skips entirely when no task is
/// bound — we don't want to burn API rate limit polling PRs nobody is tracking.
///
/// A skip is `Ok(())`; a failure that stopped the check from happening at all
/// is an error, so the work is retried rather than silently dropped the way the
/// spawned version dropped it.
pub(super) async fn check_pr_merge_conflict(
    state: &AppState,
    connection: &AdapterConnection,
    rule_for_parse: &SubscriptionMatch,
    owner: &str,
    repo: &str,
    pr_number: u64,
    sender_login: &str,
) -> Result<(), AdapterError> {
    let subject_key = format!("{owner}/{repo}#{pr_number}");

    // Skip the API poll unless some active task is actually bound to this PR.
    if !PgTaskBindingRepository(state.db.clone())
        .find_by_subject("github", "github_pr", &subject_key)
        .await?
        .iter()
        .any(|b| b.active)
    {
        debug!(
            subject_key,
            "No active bindings; skipping merge-conflict check"
        );
        return Ok(());
    }

    let Some(adapter) = state.adapters.as_ref().and_then(|a| a.github_pr()) else {
        return Ok(());
    };

    // Resolve the acting user the same way CI notifications do: the triggering
    // sender if mapped, else the rule's effective user. Used both to mint the
    // API token and as the message author.
    let user_id = resolve_user_or_fallback(sender_login, rule_for_parse, &state.db).await;
    let token = adapter
        .resolve_token_for_user(&user_id, Some(owner))
        .await?;

    let pr = GithubPrApi::new(&token)
        .get_pr_mergeable(owner, repo, pr_number)
        .await?;

    // `unknown` means GitHub never finished computing within our budget —
    // inconclusive, so leave the persisted value untouched and stay quiet.
    let mergeable = pr
        .mergeable_state
        .as_deref()
        .map(MergeableState::from_github);
    let Some(mergeable) = mergeable.filter(|m| m.is_settled()) else {
        debug!(
            subject_key,
            mergeable_state = ?pr.mergeable_state,
            "Mergeable state unresolved; skipping persist + notify"
        );
        return Ok(());
    };

    // Persist the mergeable state onto every binding, and — only on an open PR
    // transitioning *into* conflict — deliver a one-shot conflict notification.
    // Delivery is the ordinary route: the alert names its audience (the task
    // whose pull request stopped merging) and reaches it on that, not on who
    // GitHub says the sender is.
    let pr_is_open = pr.state.eq_ignore_ascii_case("open");
    let conflict_msg = InboundMessage {
        provider: AdapterKind::GithubPr,
        source_id: rule_for_parse.source_matcher.clone(),
        thread_id: Some(pr_number.to_string()),
        message_thread_id: pr_number.to_string(),
        user_id,
        text: merge_conflict_text(owner, repo, pr_number, pr.html_url.as_deref().unwrap_or("")),
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr_number": pr_number,
        }),
        provider_context: serde_json::json!({}),
        external_event_id: Some(super::external_id::merge_conflict(pr_number, &pr.head.sha)),
        source_task_id: None,
        // Whether the pull request still merges cleanly is its author's
        // concern.
        audience: Audience::SubjectProducer,
    };

    super::runtime::record_merge_state_and_notify(
        state,
        connection,
        rule_for_parse,
        &subject_key,
        mergeable,
        pr_is_open,
        &conflict_msg,
    )
    .await;
    Ok(())
}

/// Conclusions of a `workflow_run` worth announcing after a merge. Skips noise
/// (`skipped`, `neutral`, `stale`) and in-progress/absent conclusions.
const POST_MERGE_NOTIFY_CONCLUSIONS: &[&str] = &[
    "success",
    "failure",
    "timed_out",
    "cancelled",
    "action_required",
    "startup_failure",
];

/// Parsed fields of a base-branch `workflow_run.completed` event.
///
/// Serializable because it is carried as the payload of a `github.post_merge`
/// Adapter delivery record (see [`super::delivery`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PostMergeRun {
    run_id: u64,
    name: String,
    conclusion: String,
    head_sha: String,
    head_branch: String,
    html_url: String,
}

/// Extract a post-merge `workflow_run` from a webhook payload, or `None` when
/// the event is not a completed push-triggered run worth announcing.
///
/// Gates cheaply (no API calls): the run must be `completed`, triggered by a
/// `push` (a merge to the base branch — PR-branch runs are `pull_request` and
/// already covered by `check_run`/`check_suite`), carry no associated PRs in
/// its payload (present only for PR runs), and have an announce-worthy
/// conclusion. The base-branch confirmation + PR resolution happen later, in
/// the spawned task, since they require an API token.
fn parse_post_merge_run(payload: &serde_json::Value) -> Option<PostMergeRun> {
    let action = payload.get("action").and_then(serde_json::Value::as_str)?;
    if action != "completed" {
        return None;
    }
    let wr = payload.get("workflow_run")?;

    if wr.get("event").and_then(serde_json::Value::as_str) != Some("push") {
        return None;
    }
    // PR-triggered runs list their PRs here; a base-branch push run does not.
    if wr
        .get("pull_requests")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|prs| !prs.is_empty())
    {
        return None;
    }

    let conclusion = wr
        .get("conclusion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !POST_MERGE_NOTIFY_CONCLUSIONS.contains(&conclusion) {
        return None;
    }

    let head_sha = wr
        .get("head_sha")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if head_sha.is_empty() {
        return None;
    }

    Some(PostMergeRun {
        run_id: wr
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        name: wr
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("workflow")
            .to_string(),
        conclusion: conclusion.to_string(),
        head_sha: head_sha.to_string(),
        head_branch: wr
            .get("head_branch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        html_url: wr
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Render the post-merge CI notification body.
fn post_merge_ci_text(run: &PostMergeRun, owner: &str, repo: &str, pr_number: u64) -> String {
    let success = run.conclusion.eq_ignore_ascii_case("success");
    let heading = if success {
        format!("## ✅ Post-merge CI passed: {}", run.name)
    } else {
        format!("## ❌ Post-merge CI {}: {}", run.conclusion, run.name)
    };
    let short_sha = &run.head_sha[..7.min(run.head_sha.len())];
    let mut text = format!(
        "{heading}\n\n\
         The `{branch}` branch build for merged PR {owner}/{repo}#{pr_number} \
         finished with **{conclusion}**.\n\
         **Commit:** `{short_sha}`\n",
        branch = run.head_branch,
        conclusion = run.conclusion,
    );
    if !run.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", run.html_url);
    }
    text
}

/// Post-merge CI notification: on a base-branch `workflow_run` completing,
/// resolve the merged PR from the commit sha and notify every task still
/// connected to that PR.
///
/// Runs as a `github.post_merge` Adapter delivery, so it is off GitHub's
/// response deadline *and* survives a restart between acknowledgment and
/// execution. Skips entirely when the run isn't on the repo's default branch,
/// when no merged PR produced the commit, or when no active task is bound to
/// the PR.
///
/// A skip is `Ok(())`; a failure that stopped the notice from being decided at
/// all is an error, so the work is retried rather than silently dropped the way
/// the spawned version dropped it.
pub(super) async fn check_post_merge_ci(
    state: &AppState,
    connection: &AdapterConnection,
    rule_for_parse: &SubscriptionMatch,
    owner: &str,
    repo: &str,
    run: &PostMergeRun,
    sender_login: &str,
) -> Result<(), AdapterError> {
    let Some(adapter) = state.adapters.as_ref().and_then(|a| a.github_pr()) else {
        return Ok(());
    };

    // Resolve the acting user + token the same way CI/merge-conflict paths do.
    let user_id = resolve_user_or_fallback(sender_login, rule_for_parse, &state.db).await;
    let token = adapter
        .resolve_token_for_user(&user_id, Some(owner))
        .await?;
    let api = GithubPrApi::new(&token);

    // Confirm the run is on the repo's default (base) branch — post-merge runs
    // live there; runs on other branches are not merge results.
    let default_branch = api.get_repo_default_branch(owner, repo).await?;
    if default_branch != run.head_branch {
        debug!(
            head_branch = %run.head_branch,
            default_branch,
            "Skipping post-merge CI: run not on default branch"
        );
        return Ok(());
    }

    // Map the merge commit back to the PR it merged.
    let pulls = api.get_pulls_for_commit(owner, repo, &run.head_sha).await?;
    let Some(pr) = pulls.into_iter().find(super::api::CommitPr::is_merged) else {
        debug!(sha = %run.head_sha, "No merged PR for commit; skipping post-merge CI notice");
        return Ok(());
    };
    let pr_number = pr.number;
    let subject_key = format!("{owner}/{repo}#{pr_number}");

    // Skip unless some active task is still connected to the PR.
    if !PgTaskBindingRepository(state.db.clone())
        .find_by_subject("github", "github_pr", &subject_key)
        .await?
        .iter()
        .any(|b| b.active)
    {
        debug!(
            subject_key,
            "No active bindings; skipping post-merge CI notice"
        );
        return Ok(());
    }

    let notice = InboundMessage {
        provider: AdapterKind::GithubPr,
        source_id: rule_for_parse.source_matcher.clone(),
        thread_id: Some(pr_number.to_string()),
        message_thread_id: pr_number.to_string(),
        user_id,
        text: post_merge_ci_text(run, owner, repo, pr_number),
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr_number": pr_number,
        }),
        provider_context: serde_json::json!({}),
        external_event_id: Some(super::external_id::workflow_run(run.run_id)),
        source_task_id: None,
        audience: Audience::SubjectProducer,
    };

    super::runtime::deliver_post_merge_ci(state, connection, rule_for_parse, &subject_key, &notice)
        .await;
    Ok(())
}

/// Why a GitHub webhook did not reach a normal outcome.
///
/// The two variants get opposite transport treatment, which is the whole reason
/// they are distinguished: [`Self::NotAdmitted`] means the work was never taken
/// on and must not be acknowledged, while [`Self::Ingress`] keeps the
/// pre-existing behaviour of a logged, metered `200`.
#[derive(Debug, thiserror::Error)]
pub enum GithubWebhookError {
    /// Durable admission did not commit, so the backend owns nothing and a
    /// `200` would be a lie.
    ///
    /// Unlike Linear, GitHub does not retry a failed delivery on its own: the
    /// non-2xx marks it failed in the delivery log, where it stays redeliverable
    /// by an operator for 30 days. That is a recorded, recoverable loss instead
    /// of a silent one, which is the whole difference this variant buys.
    #[error("GitHub webhook work was not admitted")]
    NotAdmitted(#[from] crate::adapters::delivery::AdmitError),
    /// Everything the ingress path itself rejected: a missing header, an
    /// unparseable body, a failed signature, a database read.
    #[error(transparent)]
    Ingress(#[from] AdapterError),
}

pub async fn resolve_and_process_github_webhook(
    state: &AppState,
    github_provider: &GithubPrAdapter,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> Result<GithubWebhookOutcome, GithubWebhookError> {
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event_type.is_empty() {
        return Err(AdapterError::BadRequest("Missing X-GitHub-Event header".into()).into());
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
            "GitHub webhook ignored: no provider_connection"
        );
        return Ok(GithubWebhookOutcome::Ignored {
            event_type: event_type.to_string(),
            reason: "no_connection",
        });
    };

    // An *inactive* trigger suppresses delivery. A *missing* trigger does not —
    // events still fan out to existing PR bindings (the binding is the opt-in);
    // only new-task creation is gated on a trigger, in `route_inbound_message`.
    if rule.as_ref().is_some_and(|r| !r.active) {
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
        )
        .into());
    }

    // No delivery-level gate here on purpose. A GitHub redelivery reuses the
    // same `X-GitHub-Delivery` id and is collapsed by durable admission below —
    // on any replica, after any restart, and past any expiry. Everything this
    // function does before admitting is either read-only or derives a value
    // from the delivery payload and writes that same value again (PR status,
    // review decision), so re-running it on a redelivery costs work and changes
    // nothing.

    // `parse_webhook` only reads the trigger as a config bag (event filter,
    // source label, sender→user fallback). When no trigger matches, synthesize
    // a delivery-only one from the connection so the event is still parsed.
    // Task-creation decisions use the real `rule` below, not this fallback.
    let rule_for_parse = match &rule {
        Some(rule) => rule.clone(),
        None => super::runtime::connection_only_rule(&connection, repo_full_name),
    };

    // Shared preflight gate for `event_filter` / `ignored_senders`. `parse_webhook`
    // applies these to the primary handler, but the post-merge CI and
    // merge-conflict admissions below are derived outside its result — so
    // without this gate a trigger that filtered out an event or ignored a
    // sender could still get post-merge/conflict deliveries and wake tasks it
    // opted out of. Gating the whole dispatch here keeps trigger config
    // authoritative for every path.
    let rule_config = GithubPrRuleConfig::from_rule(&rule_for_parse);
    let sender_login = payload
        .pointer("/sender/login")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if rule_config.event_filtered_out(event_type) {
        debug!(
            event_type,
            "GitHub webhook ignored: event filtered by trigger"
        );
        return Ok(GithubWebhookOutcome::Ignored {
            event_type: event_type.to_string(),
            reason: "event_filtered",
        });
    }
    if rule_config.is_ignored_sender(sender_login) {
        debug!(
            sender_login,
            event_type, "GitHub webhook ignored: sender in ignored_senders"
        );
        return Ok(GithubWebhookOutcome::Ignored {
            event_type: event_type.to_string(),
            reason: "ignored_sender",
        });
    }

    let result = github_provider
        .parse_webhook(event_type, &payload, &rule_for_parse, &state.db)
        .await?;

    // From here on, nothing runs inline and nothing is spawned. Each kind this
    // delivery fans into is written to `adapter_delivery` under its own
    // Adapter delivery record, keyed on GitHub's delivery id, and only then is
    // the delivery acknowledged. An admission
    // failure aborts the whole dispatch with `NotAdmitted` so a delivery the
    // backend has not taken on is never acknowledged as if it had.
    let delivery_context = super::delivery::DeliveryContext {
        connection_id: connection.id.clone(),
        trigger_id: rule.as_ref().map(|r| r.id.clone()),
        repo_full_name: repo_full_name.to_string(),
    };
    let event_id = super::delivery::delivery_event_id(delivery_id, body);

    match result {
        GithubPrWebhookResult::Message(msg) => {
            admit(
                state,
                super::delivery::kind::PR_MESSAGE,
                &event_id,
                &super::delivery::PrMessageDelivery {
                    context: delivery_context.clone(),
                    message: *msg,
                },
            )
            .await?;
        }
        GithubPrWebhookResult::CloseTask(info) => {
            // Auto-close only applies to trigger-created tasks; with no trigger
            // there is nothing to close.
            if rule.is_none() {
                debug!(
                    repo = repo_full_name,
                    "PR closed but no trigger for this repo — nothing to auto-close"
                );
                return Ok(GithubWebhookOutcome::Processed {
                    event_type: event_type.to_string(),
                });
            }
            admit(
                state,
                super::delivery::kind::CLOSE,
                &event_id,
                &super::delivery::CloseDelivery {
                    context: delivery_context.clone(),
                    close: info,
                },
            )
            .await?;
        }
        GithubPrWebhookResult::Ignore => {
            debug!(event_type, "GitHub PR event ignored");
        }
    }

    // Post-merge CI: a base-branch `workflow_run` completing carries no PR
    // reference, so it never reaches the PR-keyed handlers above. Resolve the
    // merged PR from the commit sha and notify its still-connected tasks.
    // Admitted rather than run here because it makes several GitHub API calls;
    // deduped per run id downstream.
    if event_type == "workflow_run" {
        if let Some(run) = parse_post_merge_run(&payload) {
            let (owner, repo) = match repo_full_name.split_once('/') {
                Some((o, r)) => (o.to_string(), r.to_string()),
                None => (owner_login.to_string(), String::new()),
            };
            admit(
                state,
                super::delivery::kind::POST_MERGE,
                &event_id,
                &super::delivery::PostMergeDelivery {
                    context: delivery_context.clone(),
                    owner,
                    repo,
                    run,
                    sender_login: sender_login.to_string(),
                },
            )
            .await?;
        }
    }

    // Merge-conflict check, independent of the primary event's outcome: any
    // event that names a PR re-checks its mergeable state, since a conflict can
    // appear out of band (e.g. the base branch moved) with no dedicated
    // webhook. Deduped per head sha downstream, and a no-op when no task is
    // bound to the PR.
    if let Some(pr_number) = merge_conflict_pr_number(event_type, &payload) {
        let (owner, repo) = match repo_full_name.split_once('/') {
            Some((o, r)) => (o.to_string(), r.to_string()),
            None => (owner_login.to_string(), String::new()),
        };
        admit(
            state,
            super::delivery::kind::MERGE_CONFLICT,
            &event_id,
            &super::delivery::MergeConflictDelivery {
                context: delivery_context,
                owner,
                repo,
                pr_number,
                sender_login: sender_login.to_string(),
            },
        )
        .await?;
    }

    Ok(GithubWebhookOutcome::Processed {
        event_type: event_type.to_string(),
    })
}

/// Hand one delivery to durable admission, or refuse it.
///
/// A failure leaves nothing behind: no record was committed and nothing in this
/// process remembers the delivery, so the operator redelivery the resulting
/// non-2xx exists to invite is processed like any other delivery — on this
/// replica or any other.
async fn admit(
    state: &AppState,
    kind: &'static str,
    event_id: &str,
    payload: &impl serde::Serialize,
) -> Result<(), GithubWebhookError> {
    let request = super::delivery::request(kind, event_id.to_string(), payload)?;
    let admission = crate::adapters::ingress::admit_delivery(state, request).await?;
    super::delivery::log_admission(kind, event_id, admission);
    Ok(())
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
            "https://app.example.com".to_string(),
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
            "https://app.example.com".to_string(),
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
            body: "rename this",
        });
        assert!(text.contains("**@alice** commented on `src/lib.rs` (line 10):"));
        assert!(text.contains("https://github.com/o/r/pull/42#discussion_r777"));
        assert!(text.contains("Comment id: 777"));
        assert!(!text.contains("```diff"));
        assert!(text.ends_with("rename this"));
    }

    #[test]
    fn review_text_carries_review_id_and_inline_comments_hint() {
        let text = review_text("alice", "Changes Requested", 555, "see notes", "o", "r", 42);
        assert!(text.starts_with("**@alice** — Changes Requested (review id: 555):\n\nsee notes"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42/reviews/555/comments`"));
    }

    #[test]
    fn review_text_renders_placeholder_for_empty_body() {
        // A bare approval (no summary text) must still read cleanly.
        let text = review_text("alice", "Approved", 555, "", "o", "r", 42);
        assert!(
            text.starts_with("**@alice** — Approved (review id: 555):\n\n_(no review comment)_")
        );
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn merge_conflict_text_points_at_live_diff() {
        let text = merge_conflict_text("o", "r", 42, "https://github.com/o/r/pull/42");
        assert!(text.contains("## ⚠️ Merge Conflicts on PR #42"));
        assert!(text.contains("**Link:** https://github.com/o/r/pull/42"));
        assert!(text.contains("`moat gh pr diff 42`"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42`"));
    }

    #[test]
    fn merge_conflict_text_omits_link_when_url_missing() {
        let text = merge_conflict_text("o", "r", 7, "");
        assert!(!text.contains("**Link:**"));
    }

    #[test]
    fn merge_conflict_pr_number_extracts_per_event_type() {
        // pull_request (non-closed) → head PR number
        let pr = serde_json::json!({"action": "synchronize", "pull_request": {"number": 12}});
        assert_eq!(merge_conflict_pr_number("pull_request", &pr), Some(12));

        // pull_request closed → skipped (no mergeable state to report)
        let closed = serde_json::json!({"action": "closed", "pull_request": {"number": 12}});
        assert_eq!(merge_conflict_pr_number("pull_request", &closed), None);

        // review / review_comment → PR number
        let review = serde_json::json!({"pull_request": {"number": 34}});
        assert_eq!(
            merge_conflict_pr_number("pull_request_review", &review),
            Some(34)
        );
        assert_eq!(
            merge_conflict_pr_number("pull_request_review_comment", &review),
            Some(34)
        );

        // issue_comment on a PR → number; on a plain issue → None
        let pr_comment = serde_json::json!({"issue": {"number": 5, "pull_request": {"url": "x"}}});
        assert_eq!(
            merge_conflict_pr_number("issue_comment", &pr_comment),
            Some(5)
        );
        let issue_comment = serde_json::json!({"issue": {"number": 5}});
        assert_eq!(
            merge_conflict_pr_number("issue_comment", &issue_comment),
            None
        );

        // check_run / check_suite → first associated PR
        let check = serde_json::json!({"check_run": {"pull_requests": [{"number": 9}]}});
        assert_eq!(merge_conflict_pr_number("check_run", &check), Some(9));
        let suite = serde_json::json!({"check_suite": {"pull_requests": [{"number": 9}]}});
        assert_eq!(merge_conflict_pr_number("check_suite", &suite), Some(9));

        // unsupported event → None
        assert_eq!(merge_conflict_pr_number("push", &pr), None);
    }

    fn post_merge_payload(
        action: &str,
        event: &str,
        conclusion: &str,
        prs: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "action": action,
            "workflow_run": {
                "id": 555,
                "name": "CD - Publish",
                "event": event,
                "conclusion": conclusion,
                "head_sha": "12b2f924aa30",
                "head_branch": "main",
                "html_url": "https://github.com/o/r/actions/runs/555",
                "pull_requests": prs,
            }
        })
    }

    #[test]
    fn parse_post_merge_run_accepts_completed_push_run() {
        let payload = post_merge_payload("completed", "push", "success", serde_json::json!([]));
        let run = parse_post_merge_run(&payload).expect("should parse");
        assert_eq!(run.run_id, 555);
        assert_eq!(run.conclusion, "success");
        assert_eq!(run.head_branch, "main");
        assert_eq!(run.head_sha, "12b2f924aa30");
    }

    #[test]
    fn parse_post_merge_run_rejects_non_completed_or_non_push_or_pr_runs() {
        // not completed
        assert!(parse_post_merge_run(&post_merge_payload(
            "requested",
            "push",
            "success",
            serde_json::json!([])
        ))
        .is_none());
        // pull_request-triggered run (covered by check_run/check_suite already)
        assert!(parse_post_merge_run(&post_merge_payload(
            "completed",
            "pull_request",
            "success",
            serde_json::json!([])
        ))
        .is_none());
        // push run that still lists PRs → not a base-branch merge result
        assert!(parse_post_merge_run(&post_merge_payload(
            "completed",
            "push",
            "success",
            serde_json::json!([{"number": 7}])
        ))
        .is_none());
        // noise conclusion
        assert!(parse_post_merge_run(&post_merge_payload(
            "completed",
            "push",
            "skipped",
            serde_json::json!([])
        ))
        .is_none());
    }

    #[test]
    fn post_merge_ci_text_reflects_success_and_failure() {
        let ok = PostMergeRun {
            run_id: 1,
            name: "CI".into(),
            conclusion: "success".into(),
            head_sha: "abcdef012345".into(),
            head_branch: "main".into(),
            html_url: "https://x/run/1".into(),
        };
        let text = post_merge_ci_text(&ok, "o", "r", 42);
        assert!(text.contains("✅ Post-merge CI passed: CI"));
        assert!(text.contains("o/r#42"));
        assert!(text.contains("`abcdef0`"));
        assert!(text.contains("**Link:** https://x/run/1"));

        let bad = PostMergeRun {
            conclusion: "failure".into(),
            html_url: String::new(),
            ..ok
        };
        let text = post_merge_ci_text(&bad, "o", "r", 42);
        assert!(text.contains("❌ Post-merge CI failure: CI"));
        assert!(!text.contains("**Link:**"));
    }

    #[test]
    fn pr_info_deserializes_mergeable_fields() {
        let pr: crate::adapters::github_pr::api::PrInfo =
            serde_json::from_value(serde_json::json!({
                "number": 1,
                "title": "t",
                "body": null,
                "state": "open",
                "mergeable": false,
                "mergeable_state": "dirty",
                "user": {"login": "alice"},
                "head": {"ref": "feat", "sha": "abc"},
                "base": {"ref": "main", "sha": "def"},
            }))
            .expect("PrInfo parses mergeable fields");
        assert_eq!(pr.mergeable, Some(false));
        assert_eq!(pr.mergeable_state.as_deref(), Some("dirty"));

        // Fields absent (list/webhook payloads) → None, not an error.
        let pr2: crate::adapters::github_pr::api::PrInfo =
            serde_json::from_value(serde_json::json!({
                "number": 2,
                "title": "t",
                "body": null,
                "state": "open",
                "user": {"login": "bob"},
                "head": {"ref": "h", "sha": "s"},
                "base": {"ref": "b", "sha": "s2"},
            }))
            .expect("PrInfo parses without mergeable fields");
        assert_eq!(pr2.mergeable, None);
        assert_eq!(pr2.mergeable_state, None);
    }

    #[test]
    fn commit_pr_merged_detection() {
        let merged: crate::adapters::github_pr::api::CommitPr =
            serde_json::from_value(serde_json::json!({
                "number": 344,
                "state": "closed",
                "merged_at": "2026-07-17T04:00:00Z",
            }))
            .expect("CommitPr parses");
        assert!(merged.is_merged());
        assert_eq!(merged.number, 344);

        let open: crate::adapters::github_pr::api::CommitPr =
            serde_json::from_value(serde_json::json!({ "number": 7, "state": "open" }))
                .expect("CommitPr parses without merged_at");
        assert!(!open.is_merged());
    }

    #[tokio::test]
    async fn reply_instructions_cover_reply_paths_and_completion_behavior() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
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
            "https://app.example.com".to_string(),
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

    fn fake_rule() -> SubscriptionMatch {
        use crate::adapters::models::RoutingMode;
        use chrono::Utc;
        use moatless_backend_core::scope::Scope;
        SubscriptionMatch {
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
            owner_user_id: Some("alice".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Build a `TaskBinding` whose `filter_user_id` is `filter` (stored as JSON
    /// `null` when `None`), written into both `destination_config` and `metadata`
    /// to mirror `upsert_binding` (`backend/src/bindings/runtime.rs`).
    fn binding_with_filter(filter: Option<&str>) -> TaskBinding {
        use chrono::Utc;
        let filter_json = serde_json::json!({ "filter_user_id": filter });
        TaskBinding {
            id: "tb_test".into(),
            task_id: "task_test".into(),
            provider: "github_pr".into(),
            subject_kind: "github_pr".into(),
            subject_key: "soaplabs/repo#42".into(),
            connection_id: "pc_test".into(),
            trigger_id: None,
            destination_config: filter_json.clone(),
            metadata: filter_json,
            last_external_event_id: None,
            origin: "connect".into(),
            created_at: Utc::now(),
            last_event_at: Utc::now(),
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    #[test]
    fn outbound_user_id_prefers_task_owner_over_filter_and_rule() {
        let binding = binding_with_filter(Some("someone-else"));
        let mut rule = fake_rule();
        rule.run_as_user_id = Some("bot:reviewer".into());
        assert_eq!(
            GithubPrAdapter::outbound_user_id(Some("195784656"), &binding, Some(&rule)),
            Some("195784656".to_string())
        );
    }

    #[test]
    fn outbound_user_id_falls_back_to_binding_filter_when_no_owner() {
        let binding = binding_with_filter(Some("195784656"));
        let mut rule = fake_rule();
        rule.run_as_user_id = Some("bot:reviewer".into());
        assert_eq!(
            GithubPrAdapter::outbound_user_id(None, &binding, Some(&rule)),
            Some("195784656".to_string())
        );
    }

    #[test]
    fn outbound_user_id_falls_back_to_rule_when_no_owner_or_filter() {
        let binding = binding_with_filter(None);
        let mut rule = fake_rule();
        rule.run_as_user_id = Some("bot:reviewer".into());
        assert_eq!(
            GithubPrAdapter::outbound_user_id(None, &binding, Some(&rule)),
            Some("bot:reviewer".to_string())
        );
    }

    #[test]
    fn outbound_user_id_none_when_nothing_present() {
        let binding = binding_with_filter(None);
        assert_eq!(
            GithubPrAdapter::outbound_user_id(None, &binding, None),
            None
        );
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
            "https://app.example.com".to_string(),
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let mut rule = fake_rule();
        // Mixed case in config vs payload — match must be case-insensitive.
        rule.provider_config = Some(serde_json::json!({"ignored_senders": ["Linear-Code[bot]"]}));
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
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
    async fn linear_code_bot_dropped_by_default() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
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
            "https://app.example.com".to_string(),
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

    fn pr_opened_payload(title: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "opened",
            "sender": { "login": "alice" },
            "repository": { "full_name": "soaplabs/repo" },
            "pull_request": {
                "number": 42,
                "title": title,
                "html_url": "https://github.com/soaplabs/repo/pull/42",
                "head": { "ref": "feature" },
                "base": { "ref": "main" },
            }
        })
    }

    #[test]
    fn title_is_ignored_matches_case_insensitively_and_skips_bad_regex() {
        let patterns = vec!["^chore\\(deps".to_string(), "^deps[:(]".to_string()];
        assert!(title_is_ignored("chore(deps): bump x", &patterns));
        assert!(title_is_ignored("CHORE(deps): bump x", &patterns));
        assert!(title_is_ignored("deps(frontend): bump y", &patterns));
        assert!(!title_is_ignored("feat: real feature", &patterns));
        // An invalid regex is skipped, not fatal — and matches nothing.
        assert!(!title_is_ignored("anything", &["(".to_string()]));
    }

    #[tokio::test]
    async fn pr_opened_with_matching_title_is_ignored() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let mut rule = fake_rule();
        rule.provider_config =
            Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps", "^deps[:(]"]}));
        let db = fake_db_pool();

        for title in ["chore(deps): bump serde", "deps(frontend): bump vite"] {
            let result = provider
                .parse_webhook("pull_request", &pr_opened_payload(title), &rule, &db)
                .await
                .unwrap();
            assert!(
                matches!(result, GithubPrWebhookResult::Ignore),
                "expected Ignore for title {title:?}"
            );
        }
    }

    #[tokio::test]
    async fn pr_opened_with_non_matching_title_delivers() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        let mut rule = fake_rule();
        rule.provider_config =
            Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps"]}));
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
                "pull_request",
                &pr_opened_payload("feat: add a real feature"),
                &rule,
                &db,
            )
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Message(_)));
    }

    #[tokio::test]
    async fn pr_opened_without_patterns_delivers() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
            fake_db_pool(),
            fake_provider_tokens(),
            None,
            None,
            fake_github_apps(),
        );
        // No provider_config — title filtering is opt-in, so this is delivered.
        let rule = fake_rule();
        let db = fake_db_pool();

        let result = provider
            .parse_webhook(
                "pull_request",
                &pr_opened_payload("chore(deps): bump serde"),
                &rule,
                &db,
            )
            .await
            .unwrap();

        assert!(matches!(result, GithubPrWebhookResult::Message(_)));
    }

    #[tokio::test]
    async fn issue_comment_with_footer_link_delivers_with_source_task_id() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
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
                "body": "Acknowledged. Waiting for a fix commit.\n\n---\nMoatless task: [n](https://app.example.com/tasks/task-A)",
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
            "https://app.example.com".to_string(),
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
    async fn review_comment_with_footer_link_delivers_with_source_task_id() {
        let provider = GithubPrAdapter::new(
            None,
            None,
            "https://app.example.com".to_string(),
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
                "body": "Acknowledged on the inline thread.\n\n---\nMoatless task: [n](https://app.example.com/tasks/task-B)",
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

    /// The audience each event states is the whole of what this adapter tells
    /// the router about who an event is for. It is the seam: above it the
    /// adapter knows GitHub's event shapes, below it the router knows bindings,
    /// and neither needs the other's vocabulary. These pin the classification
    /// so a shape cannot quietly change hands.
    mod audience {
        use super::*;

        fn adapter() -> GithubPrAdapter {
            GithubPrAdapter::new(
                None,
                None,
                "https://app.example.com".to_string(),
                fake_db_pool(),
                fake_provider_tokens(),
                None,
                None,
                fake_github_apps(),
            )
        }

        async fn audience_of(
            event_type: &str,
            payload: &serde_json::Value,
        ) -> Option<crate::adapters::models::Audience> {
            let provider = adapter();
            let mut rule = fake_rule();
            rule.source_matcher = "soaplabs/repo".to_string();
            let db = fake_db_pool();
            let ctx = EventContext {
                rule: &rule,
                owner: "soaplabs",
                repo: "repo",
                sender_login: "alice",
                db: &db,
            };
            let result = match event_type {
                "issue_comment" => provider.handle_issue_comment_event(payload, &ctx).await,
                "pull_request_review_comment" => {
                    provider.handle_review_comment_event(payload, &ctx).await
                }
                "pull_request_review" => provider.handle_review_event(payload, &ctx).await,
                "check_run" => provider.handle_check_run_event(payload, &ctx).await,
                other => panic!("unhandled event type {other}"),
            }
            .expect("the event parses");
            match result {
                GithubPrWebhookResult::Message(msg) => Some(msg.audience),
                _ => None,
            }
        }

        fn review_comment(in_reply_to_id: Option<u64>) -> serde_json::Value {
            serde_json::json!({
                "action": "created",
                "pull_request": { "number": 42 },
                "comment": {
                    "id": 999_u64,
                    "body": "This early-returns on an empty slice.",
                    "path": "src/foo.rs",
                    "html_url": "https://github.com/soaplabs/repo/pull/42#discussion_r999",
                    "in_reply_to_id": in_reply_to_id,
                }
            })
        }

        /// The event that was lost: an inline comment starting its own thread.
        /// It names no parent, so no responder is its audience — and the
        /// producing task is left as the recipient, whoever posted it.
        #[tokio::test]
        async fn an_inline_comment_starting_a_thread_names_no_parent() {
            assert_eq!(
                audience_of("pull_request_review_comment", &review_comment(None)).await,
                Some(Audience::ParentAuthor {
                    parent_external_event_id: None
                })
            );
        }

        /// A reply names the comment it answers, so the task that wrote that
        /// comment receives it — alongside the producer, by the resolver's rule.
        #[tokio::test]
        async fn a_reply_inside_a_thread_names_the_comment_it_answers() {
            assert_eq!(
                audience_of("pull_request_review_comment", &review_comment(Some(555))).await,
                Some(Audience::ParentAuthor {
                    parent_external_event_id: Some(super::super::super::external_id::review_comment(
                        555
                    ))
                })
            );
        }

        /// A review submission is about the pull request as a whole, not about
        /// one thread, so every task bound to it hears the verdict.
        #[tokio::test]
        async fn a_review_submission_is_for_everyone_bound() {
            let payload = serde_json::json!({
                "action": "submitted",
                "pull_request": { "number": 42 },
                "review": {
                    "id": 77_u64,
                    "state": "changes_requested",
                    "body": "Two things.",
                }
            });
            assert_eq!(
                audience_of("pull_request_review", &payload).await,
                Some(Audience::Everyone)
            );
        }

        #[tokio::test]
        async fn a_comment_on_the_pull_request_itself_is_for_everyone_bound() {
            let payload = serde_json::json!({
                "action": "created",
                "issue": {
                    "number": 42,
                    "pull_request": { "url": "https://api.github.com/repos/soaplabs/repo/pulls/42" }
                },
                "comment": { "id": 123_u64, "body": "Rebased." }
            });
            assert_eq!(
                audience_of("issue_comment", &payload).await,
                Some(Audience::Everyone)
            );
        }

        /// A build result belongs to whoever produced the pull request. It used
        /// to need an exemption from the owner-identity guard to arrive at all;
        /// stating the audience is what replaces it.
        #[tokio::test]
        async fn a_build_result_is_for_the_pull_requests_producer() {
            let payload = serde_json::json!({
                "action": "completed",
                "check_run": {
                    "id": 31_u64,
                    "name": "CI",
                    "conclusion": "failure",
                    "html_url": "https://github.com/soaplabs/repo/runs/31",
                    "pull_requests": [{ "number": 42 }],
                    "output": { "summary": "1 test failed" },
                }
            });
            assert_eq!(
                audience_of("check_run", &payload).await,
                Some(Audience::SubjectProducer)
            );
        }
    }
}

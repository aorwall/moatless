//! Raw GitHub webhook normalization.
//!
//! This module interprets event/action payloads and returns values for the
//! current ingress coordinator to project or admit. Sender-to-user resolution
//! is the only I/O performed here.

use tracing::{debug, info, warn};

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::SubscriptionMatch;

use super::config::GithubPrRuleConfig;
use super::signals::ReviewDecision;
use super::status::{PullRequestFields, WebhookStatusObservation, github_external_status};
use super::subject::GithubPrSubject;
use super::text::{
    CheckRunFailedText, PrOpenedText, ReviewCommentText, check_run_failed_text, issue_comment_text,
    pr_opened_text, pr_synchronized_text, review_comment_text, review_text,
};

/// Everything downstream workflows can derive from one raw webhook.
pub(super) struct NormalizedWebhook {
    pub(super) status_observations: Vec<WebhookStatusObservation>,
    pub(super) primary: Result<PrimaryWebhook, AdapterError>,
    pub(super) merge_conflict_candidate: Option<super::merge_conflict::Candidate>,
    pub(super) post_merge_candidate: Option<super::post_merge::PostMergeRun>,
    pub(super) merged_candidate: Option<super::merged::MergedPr>,
    /// Every closed pull request, whatever the trigger's filter and
    /// `close_task_on_close` say, so an opted-in Binding's Task closes too.
    pub(super) closed_candidate: Option<PrCloseInfo>,
}

/// The primary durable effect, if any, of a webhook.
#[derive(Debug)]
pub(super) enum PrimaryWebhook {
    Message(Box<InboundMessage>),
    /// A notice that still needs a GitHub read before it can be delivered, kept
    /// apart from [`Self::Message`] so the read happens under the delivery
    /// record's retries rather than inside the webhook request.
    CheckRun(Box<super::check_run::Execution>),
    /// A check run matching a `create_on: check_run` loop's trigger; the pull
    /// request is read and the Task opened under the delivery record's retries.
    CheckRunTrigger(Box<super::check_run_trigger::Execution>),
    CloseTask(PrCloseInfo),
    Ignore(IgnoredWebhook),
}

/// Stable normalization reasons retained for ingress metrics and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IgnoredWebhook {
    EventFiltered,
    IgnoredSender,
    UnsupportedEvent,
    UnsupportedAction,
    NonPullRequestIssue,
    IgnoredTitle,
    EmptyReview,
    NotificationDisabled,
    NonNotifiableConclusion,
    MissingPullRequest,
    /// A trusted trigger check run whose marker is unreadable, names a bad
    /// skill or another head, or that names no pull request.
    InvalidCheckRunTrigger,
}

impl IgnoredWebhook {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::EventFiltered => "event_filtered",
            Self::IgnoredSender => "ignored_sender",
            Self::UnsupportedEvent => "unsupported_event",
            Self::UnsupportedAction => "unsupported_action",
            Self::NonPullRequestIssue => "non_pull_request_issue",
            Self::IgnoredTitle => "ignored_title",
            Self::EmptyReview => "empty_review",
            Self::NotificationDisabled => "notification_disabled",
            Self::NonNotifiableConclusion => "non_notifiable_conclusion",
            Self::MissingPullRequest => "missing_pull_request",
            Self::InvalidCheckRunTrigger => "invalid_check_run_trigger",
        }
    }
}

/// Metadata about a closed PR, carried by a `github.close` delivery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PrCloseInfo {
    pub(super) pr_number: u64,
    pub(super) merged: bool,
    pub(super) title: String,
    pub(super) html_url: String,
    pub(super) sender_login: String,
    #[serde(default)]
    pub(super) pr_author_login: Option<String>,
}

/// Facts already validated by ingress and configuration needed to normalize.
pub(super) struct NormalizeContext<'a> {
    pub(super) rule: &'a SubscriptionMatch,
    pub(super) owner: &'a str,
    pub(super) repo: &'a str,
    pub(super) sender_login: &'a str,
    /// Who opened the pull request, from [`pr_author_login`]. Stands in for a
    /// sender with no moatless identity.
    pub(super) pr_author_login: Option<&'a str>,
    pub(super) frontend_url: &'a str,
    pub(super) users: &'a dyn ProviderIdentityLink,
}

/// The login of whoever opened the pull request this event happened on.
///
/// A `pull_request`-family payload carries the pull request and an
/// `issue_comment` the issue that is the same row; a `check_run` carries
/// neither, so it has no author to fall back to.
pub(super) fn pr_author_login(payload: &serde_json::Value) -> Option<&str> {
    payload
        .pointer("/pull_request/user/login")
        .or_else(|| payload.pointer("/issue/user/login"))
        .and_then(serde_json::Value::as_str)
}

/// What routing filters this webhook's primary message by; see
/// [`super::config::FilterKey`].
pub(super) fn filter_key(
    event_type: &str,
    payload: &serde_json::Value,
    sender_login: &str,
) -> super::config::FilterKey {
    super::config::FilterKey {
        approval: event_type == "pull_request_review"
            && payload
                .pointer("/review/state")
                .and_then(serde_json::Value::as_str)
                == Some("approved"),
        ..super::config::FilterKey::new(filter_event(event_type, payload), sender_login)
    }
}

/// The [`super::config::KNOWN_EVENTS`] name a loop's filters match this
/// webhook's primary message under: [`super::config::REVIEW_VERDICT`] for a
/// body-less approval or change request, else `event_type`.
fn filter_event<'a>(event_type: &'a str, payload: &serde_json::Value) -> &'a str {
    let review = |key: &str| {
        payload
            .pointer(&format!("/review/{key}"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    if event_type == "pull_request_review"
        && review("body").is_empty()
        && matches!(review("state"), "approved" | "changes_requested")
    {
        super::config::REVIEW_VERDICT
    } else {
        event_type
    }
}

impl NormalizeContext<'_> {
    /// The pull request an event in this delivery happened on. Every id and
    /// subject key built here names the repository, so a number is never on its
    /// own.
    fn subject(&self, pr_number: u64) -> GithubPrSubject {
        GithubPrSubject::new(self.owner, self.repo, pr_number)
    }
}

/// Normalize one GitHub webhook without persistence, API calls, admission, or
/// routing.
pub(super) async fn normalize(
    event_type: &str,
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> NormalizedWebhook {
    let config = GithubPrRuleConfig::from_rule(context.rule);
    let unknown = config.unknown_event_names();
    if !unknown.is_empty() {
        warn!(
            loop_id = %context.rule.id,
            ?unknown,
            known = ?super::config::KNOWN_EVENTS,
            "Loop names events that do not exist; they filter nothing"
        );
    }
    let inert = super::config::inert_filter_entries(context.rule.adapter_config.as_ref());
    if !inert.is_empty() {
        warn!(
            loop_id = %context.rule.id,
            ?inert,
            "Loop's ignored_senders or ignored_events holds entries that are not strings; \
             they filter nothing"
        );
    }
    // A default-ignored sender the loop did not opt back in to reaches no
    // Task. Any other sender the loop ignores still reaches every other Task,
    // so it gates only the loop's own effects: opening and closing its Tasks
    // here, and its Tasks' inbox in `GithubPrInboundRouter::unmuted`.
    let sender_ignored = config.is_ignored_sender(context.sender_login);
    if sender_ignored
        && super::config::FilterKey::new(event_type, context.sender_login)
            .sender_is_default_ignored()
    {
        return ignored(IgnoredWebhook::IgnoredSender);
    }

    let merge_conflict_candidate = super::merge_conflict::candidate(event_type, payload);
    let post_merge_candidate = super::post_merge::candidate(event_type, payload);
    let merged_candidate = super::merged::candidate(event_type, payload);
    let closed_candidate = (!sender_ignored)
        .then(|| closed_candidate(event_type, payload, context))
        .flatten();
    let mut status_observations = checks_observations(event_type, payload, context);

    let trigger = (!sender_ignored)
        .then(|| super::check_run_trigger::normalize(event_type, payload, context, &config))
        .flatten();
    let mut primary = if let Some(request) = trigger {
        Ok(request)
    } else if is_pr_close(event_type, payload)
        && (sender_ignored || config.event_filtered_out(event_type))
    {
        // Only the loop's own auto-close is withheld; `closed_candidate`
        // still closes the Tasks whose Binding opted in.
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered))
    } else {
        match event_type {
            "pull_request" => {
                normalize_pull_request(payload, context, &config, &mut status_observations).await
            }
            "issue_comment" => normalize_issue_comment(payload, context).await,
            "pull_request_review_comment" => normalize_review_comment(payload, context).await,
            "pull_request_review" => {
                normalize_review(payload, context, &mut status_observations).await
            }
            "check_run" => normalize_check_run(payload, context).await,
            "check_suite" => normalize_check_suite(payload, context, &config).await,
            _ => Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedEvent)),
        }
    };
    let event = super::config::inbound_event(filter_event(event_type, payload));
    match &mut primary {
        Ok(PrimaryWebhook::Message(message)) => message.event = Some(event),
        Ok(PrimaryWebhook::CheckRun(execution)) => execution.message.event = Some(event),
        _ => {}
    }

    NormalizedWebhook {
        status_observations,
        primary,
        merge_conflict_candidate,
        post_merge_candidate,
        merged_candidate,
        closed_candidate,
    }
}

/// A completed check suite, for each pull request it ran on: their checks
/// rollup is read again, whatever the Loop asks to be told of.
fn checks_observations(
    event_type: &str,
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Vec<WebhookStatusObservation> {
    if event_type != "check_suite"
        || payload.get("action").and_then(serde_json::Value::as_str) != Some("completed")
    {
        return Vec::new();
    }
    payload
        .pointer("/check_suite/pull_requests")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pr| pr.get("number").and_then(serde_json::Value::as_u64))
        .map(|number| WebhookStatusObservation::ChecksCompleted {
            subject: context.subject(number),
        })
        .collect()
}

/// Whether the webhook reports a pull request closing, merged or not.
pub(super) fn is_pr_close(event_type: &str, payload: &serde_json::Value) -> bool {
    event_type == "pull_request"
        && payload.get("action").and_then(serde_json::Value::as_str) == Some("closed")
}

/// The closed pull request, or `None` when the webhook reports no close or
/// names no PR number.
fn closed_candidate(
    event_type: &str,
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Option<PrCloseInfo> {
    if !is_pr_close(event_type, payload) {
        return None;
    }
    let string_at = |pointer: &str| {
        payload
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(PrCloseInfo {
        pr_number: payload
            .pointer("/pull_request/number")
            .and_then(serde_json::Value::as_u64)?,
        merged: payload
            .pointer("/pull_request/merged")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        title: string_at("/pull_request/title"),
        html_url: string_at("/pull_request/html_url"),
        sender_login: context.sender_login.to_string(),
        pr_author_login: context.pr_author_login.map(str::to_string),
    })
}

fn ignored(reason: IgnoredWebhook) -> NormalizedWebhook {
    NormalizedWebhook {
        status_observations: Vec::new(),
        primary: Ok(PrimaryWebhook::Ignore(reason)),
        merge_conflict_candidate: None,
        post_merge_candidate: None,
        merged_candidate: None,
        closed_candidate: None,
    }
}

const STATUS_ACTIONS: &[&str] = &[
    "opened",
    "reopened",
    "closed",
    "ready_for_review",
    "converted_to_draft",
    "edited",
    "synchronize",
];

async fn normalize_pull_request(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
    config: &GithubPrRuleConfig,
    observations: &mut Vec<WebhookStatusObservation>,
) -> Result<PrimaryWebhook, AdapterError> {
    let action = payload
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if STATUS_ACTIONS.contains(&action) {
        if let Some(observation) = pr_status_observation(payload, context) {
            observations.push(observation);
        }
    }

    if action == "synchronize" {
        return normalize_pull_request_sync(payload, context).await;
    }

    if action == "closed" {
        if config.close_task_on_close {
            let close = closed_candidate("pull_request", payload, context)
                .ok_or_else(|| SharedAdapterError::BadRequest("Missing PR number".into()))?;
            info!(
                pr_number = close.pr_number,
                merged = close.merged,
                "PR closed, will close associated tasks"
            );
            return Ok(PrimaryWebhook::CloseTask(close));
        }
        debug!("PR closed but close_task_on_close is disabled");
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::NotificationDisabled));
    }

    if action != "opened" && action != "reopened" {
        debug!(action, "Ignoring pull_request action");
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }

    let pr = payload
        .get("pull_request")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing pull_request in payload".into()))?;
    let pr_number = pr
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing PR number".into()))?;
    let title = pr
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if !config.ignored_title_patterns.is_empty()
        && title_is_ignored(title, &config.ignored_title_patterns)
    {
        info!(
            title,
            "GitHub PR skipped: title matches ignored_title_patterns"
        );
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::IgnoredTitle));
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
        sender: context.sender_login,
        head_ref,
        base_ref,
        html_url,
        owner: context.owner,
        repo: context.repo,
    });

    Ok(PrimaryWebhook::Message(Box::new(
        inbound_message(
            context,
            pr_number,
            None,
            text,
            Audience::Everyone,
            super::external_id::pr_event(action, &context.subject(pr_number), None),
        )
        .await,
    )))
}

fn pr_status_observation(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Option<WebhookStatusObservation> {
    let pr = payload.get("pull_request")?;
    let pr_number = pr.get("number").and_then(serde_json::Value::as_u64)?;
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
    let mut status = github_external_status(&PullRequestFields {
        state,
        merged,
        draft,
        title: pr.get("title").and_then(serde_json::Value::as_str),
        url: pr.get("html_url").and_then(serde_json::Value::as_str),
        base_ref: pr.pointer("/base/ref").and_then(serde_json::Value::as_str),
        head_ref: pr.pointer("/head/ref").and_then(serde_json::Value::as_str),
        updated_at: pr.get("updated_at").and_then(serde_json::Value::as_str),
    });
    // A new head has none of the old head's checks; any other action leaves
    // `checksState` out, so the merge keeps the last live read's rollup.
    if payload.get("action").and_then(serde_json::Value::as_str) == Some("synchronize") {
        status.raw["checksState"] = serde_json::Value::Null;
    }
    Some(WebhookStatusObservation::PullRequest {
        subject_key: context.subject(pr_number).key(),
        status,
    })
}

async fn normalize_pull_request_sync(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
    let pr = payload
        .get("pull_request")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing pull_request in payload".into()))?;
    let pr_number = pr
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing PR number".into()))?;
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
    let text = pr_synchronized_text(
        context.sender_login,
        pr_number,
        head_ref,
        html_url,
        before,
        after,
    );
    let sync_sha = if after.is_empty() { None } else { Some(after) };
    Ok(PrimaryWebhook::Message(Box::new(
        inbound_message(
            context,
            pr_number,
            Some(pr_number.to_string()),
            text,
            Audience::Everyone,
            super::external_id::pr_event("synchronize", &context.subject(pr_number), sync_sha),
        )
        .await,
    )))
}

async fn normalize_issue_comment(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("created") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    if payload.pointer("/issue/pull_request").is_none() {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::NonPullRequestIssue));
    }
    let pr_number = payload
        .pointer("/issue/number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing issue number".into()))?;
    let comment = payload
        .get("comment")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing comment in payload".into()))?;
    let body = comment
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let comment_id = comment
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let source_task_id =
        moatless_adapter_model::footer::parse_task_id_from_body(context.frontend_url, body);
    let mut message = inbound_message(
        context,
        pr_number,
        Some(pr_number.to_string()),
        issue_comment_text(context.sender_login, pr_number, comment_id, body),
        Audience::Everyone,
        super::external_id::issue_comment(comment_id),
    )
    .await;
    message.thread_properties["comment_id"] = serde_json::json!(comment_id);
    message.source_task_id = source_task_id;
    message.replies_to =
        super::reply_keys::answered_by_conversation_comment(body, context.sender_login);
    message.adapter_context[super::reply_keys::OPENS_KEY] = serde_json::json!(
        super::reply_keys::opened_by_conversation_comment(comment_id, body)
    );
    Ok(PrimaryWebhook::Message(Box::new(message)))
}

async fn normalize_review_comment(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("created") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    let pr_number = payload
        .pointer("/pull_request/number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing PR number".into()))?;
    let comment = payload
        .get("comment")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing comment in payload".into()))?;
    let body = comment
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let comment_id = comment
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let in_reply_to_id = comment
        .get("in_reply_to_id")
        .and_then(serde_json::Value::as_u64);
    let text = review_comment_text(&ReviewCommentText {
        sender: context.sender_login,
        path: comment
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        start_line: comment
            .get("start_line")
            .and_then(serde_json::Value::as_u64),
        line: comment.get("line").and_then(serde_json::Value::as_u64),
        html_url: comment
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        comment_id,
        in_reply_to_id,
        body,
    });
    let mut message = inbound_message(
        context,
        pr_number,
        Some(pr_number.to_string()),
        text,
        Audience::ParentAuthor {
            parent_external_event_id: in_reply_to_id.map(super::external_id::review_comment),
        },
        super::external_id::review_comment(comment_id),
    )
    .await;
    message.thread_properties["comment_id"] = serde_json::json!(comment_id);
    message.thread_properties["in_reply_to_id"] = serde_json::json!(in_reply_to_id);
    message.replies_to = super::reply_keys::answered_by_review_comment(in_reply_to_id);
    message.adapter_context[super::reply_keys::OPENS_KEY] = serde_json::json!(
        super::reply_keys::opened_by_review_comment(comment_id, in_reply_to_id)
    );
    message.source_task_id =
        moatless_adapter_model::footer::parse_task_id_from_body(context.frontend_url, body);
    Ok(PrimaryWebhook::Message(Box::new(message)))
}

async fn normalize_review(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
    observations: &mut Vec<WebhookStatusObservation>,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("submitted") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    let pr_number = payload
        .pointer("/pull_request/number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing PR number".into()))?;
    let review = payload
        .get("review")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing review in payload".into()))?;
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

    if let Some(decision) =
        ReviewDecision::from_review_state(review_state).filter(|decision| decision.is_verdict())
    {
        observations.push(WebhookStatusObservation::ReviewDecision {
            subject_key: context.subject(pr_number).key(),
            decision,
        });
    }

    let is_verdict = matches!(review_state, "approved" | "changes_requested");
    if review_body.is_empty() && !is_verdict {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::EmptyReview));
    }
    let state_label = match review_state {
        "approved" => "Approved",
        "changes_requested" => "Changes Requested",
        "commented" => "Review Comment",
        _ => "Review",
    };
    let text = review_text(
        context.sender_login,
        state_label,
        review_id,
        review_body,
        context.owner,
        context.repo,
        pr_number,
    );
    Ok(PrimaryWebhook::Message(Box::new(
        inbound_message(
            context,
            pr_number,
            Some(pr_number.to_string()),
            text,
            Audience::Everyone,
            super::external_id::review(review_id),
        )
        .await,
    )))
}

/// Conclusions worth waking a task for.
///
/// `cancelled` is deliberately absent: it's usually collateral (a concurrency
/// group superseding the run, a sibling job failing first) and says nothing the
/// task can act on.
const CHECK_RUN_FAILURE_CONCLUSIONS: &[&str] = &["failure", "timed_out", "action_required"];

async fn normalize_check_run(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("completed") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    let check_run = payload
        .get("check_run")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing check_run in payload".into()))?;
    let conclusion = check_run
        .get("conclusion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !CHECK_RUN_FAILURE_CONCLUSIONS.contains(&conclusion) {
        return Ok(PrimaryWebhook::Ignore(
            IgnoredWebhook::NonNotifiableConclusion,
        ));
    }
    let Some(pr_number) = check_run
        .pointer("/pull_requests/0/number")
        .and_then(serde_json::Value::as_u64)
    else {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::MissingPullRequest));
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
    let text = check_run_failed_text(&CheckRunFailedText {
        name,
        conclusion,
        app_name: check_run
            .pointer("/app/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("CI"),
        head_sha,
        html_url,
        output_title: check_run
            .pointer("/output/title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        output_summary: check_run
            .pointer("/output/summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    });
    // The count only: it says whether the text above is missing what
    // `api::CheckAnnotation` documents, and the delivery does the read.
    let annotations_count = check_run
        .pointer("/output/annotations_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(PrimaryWebhook::CheckRun(Box::new(
        super::check_run::Execution {
            owner: context.owner.to_string(),
            repo: context.repo.to_string(),
            check_run_id,
            annotations_count,
            message: inbound_message(
                context,
                pr_number,
                Some(pr_number.to_string()),
                text,
                Audience::SubjectProducer,
                super::external_id::check_run(&super::external_id::CheckRunKey {
                    owner: context.owner,
                    repo: context.repo,
                    head_sha,
                    app_id: check_run
                        .pointer("/app/id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    name,
                    run_id: check_run_id,
                }),
            )
            .await,
        },
    )))
}

async fn normalize_check_suite(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
    config: &GithubPrRuleConfig,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("completed") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    if !config.notify_on_success {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::NotificationDisabled));
    }
    let check_suite = payload
        .get("check_suite")
        .ok_or_else(|| SharedAdapterError::BadRequest("Missing check_suite in payload".into()))?;
    if check_suite
        .get("conclusion")
        .and_then(serde_json::Value::as_str)
        != Some("success")
    {
        return Ok(PrimaryWebhook::Ignore(
            IgnoredWebhook::NonNotifiableConclusion,
        ));
    }
    let Some(pr_number) = check_suite
        .pointer("/pull_requests/0/number")
        .and_then(serde_json::Value::as_u64)
    else {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::MissingPullRequest));
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
    Ok(PrimaryWebhook::Message(Box::new(
        inbound_message(
            context,
            pr_number,
            Some(pr_number.to_string()),
            text,
            Audience::SubjectProducer,
            super::external_id::check_suite(check_suite_id),
        )
        .await,
    )))
}

async fn inbound_message(
    context: &NormalizeContext<'_>,
    pr_number: u64,
    thread_id: Option<String>,
    text: String,
    audience: Audience,
    external_event_id: String,
) -> InboundMessage {
    let user_id = super::auth::resolve_sender_user(
        context.sender_login,
        context.pr_author_login,
        context.rule,
        context.users,
    )
    .await;
    InboundMessage {
        adapter_kind: AdapterKind::GithubPr,
        source_id: context.rule.source_matcher.clone(),
        thread_id,
        message_thread_id: pr_number.to_string(),
        user_id,
        author: moatless_types::adapter::author_handle(context.sender_login),
        sender_external_id: None,
        requester: None,
        text,
        file_ids: vec![],
        attachments: Vec::new(),
        thread_properties: serde_json::json!({
            "owner": context.owner,
            "repo": context.repo,
            "pr_number": pr_number,
        }),
        adapter_context: serde_json::json!({}),
        external_event_id: Some(external_event_id),
        source_task_id: None,
        audience,
        replies_to: Vec::new(),
        event: None,
    }
}

fn title_is_ignored(title: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        match regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(regex) => regex.is_match(title),
            Err(error) => {
                warn!(
                    pattern,
                    %error,
                    "Invalid ignored_title_patterns regex; skipping"
                );
                false
            }
        }
    })
}

#[cfg(test)]
#[path = "webhook_tests.rs"]
mod tests;

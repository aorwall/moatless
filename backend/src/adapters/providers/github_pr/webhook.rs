//! Raw GitHub webhook normalization.
//!
//! This module interprets event/action payloads and returns values for the
//! current ingress coordinator to project or admit. Sender-to-user resolution
//! is the only I/O performed here.

use std::fmt::Write;

use tracing::{debug, info, warn};

use crate::adapters::AdapterKind;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{Audience, InboundMessage, SubscriptionMatch};
use moatless_core::db::DbPool;

use super::config::GithubPrRuleConfig;
use super::signals::ReviewDecision;
use super::status::{WebhookStatusObservation, github_external_status};
use super::subject::GithubPrSubject;
use super::text::{
    PrOpenedText, ReviewCommentText, issue_comment_text, pr_opened_text, pr_synchronized_text,
    review_comment_text, review_text,
};

/// Everything downstream workflows can derive from one raw webhook.
pub(super) struct NormalizedWebhook {
    pub(super) status_observations: Vec<WebhookStatusObservation>,
    pub(super) primary: Result<PrimaryWebhook, AdapterError>,
    pub(super) merge_conflict_candidate: Option<super::merge_conflict::Candidate>,
    pub(super) post_merge_candidate: Option<super::post_merge::PostMergeRun>,
}

/// The primary durable effect, if any, of a webhook.
pub(super) enum PrimaryWebhook {
    Message(Box<InboundMessage>),
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
}

/// Facts already validated by ingress and configuration needed to normalize.
pub(super) struct NormalizeContext<'a> {
    pub(super) rule: &'a SubscriptionMatch,
    pub(super) owner: &'a str,
    pub(super) repo: &'a str,
    pub(super) sender_login: &'a str,
    pub(super) frontend_url: &'a str,
    pub(super) db: &'a DbPool,
}

/// Normalize one GitHub webhook without persistence, API calls, admission, or routing.
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
    // An ignored sender suppresses every effect of the event, derived included.
    if config.is_ignored_sender(context.sender_login) {
        return ignored(IgnoredWebhook::IgnoredSender);
    }

    // Each derived signal is gated by its own name only. The raw event is just
    // the carrier — a conflict has no webhook of its own — so filtering out the
    // carrier's message must not cancel a signal the loop asked to receive.
    let merge_conflict_candidate = (!config.event_filtered_out(super::config::MERGE_CONFLICT))
        .then(|| super::merge_conflict::candidate(event_type, payload))
        .flatten();
    let post_merge_candidate = (!config.event_filtered_out(super::config::POST_MERGE))
        .then(|| super::post_merge::candidate(event_type, payload))
        .flatten();
    let mut status_observations = Vec::new();

    let primary = if config.event_filtered_out(event_type) {
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

    NormalizedWebhook {
        status_observations,
        primary,
        merge_conflict_candidate,
        post_merge_candidate,
    }
}

fn ignored(reason: IgnoredWebhook) -> NormalizedWebhook {
    NormalizedWebhook {
        status_observations: Vec::new(),
        primary: Ok(PrimaryWebhook::Ignore(reason)),
        merge_conflict_candidate: None,
        post_merge_candidate: None,
    }
}

const STATUS_ACTIONS: &[&str] = &[
    "opened",
    "reopened",
    "closed",
    "ready_for_review",
    "converted_to_draft",
    "edited",
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
            return Ok(PrimaryWebhook::CloseTask(PrCloseInfo {
                pr_number,
                merged,
                title,
                html_url,
                sender_login: context.sender_login.to_string(),
            }));
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
        .ok_or_else(|| AdapterError::BadRequest("Missing pull_request in payload".into()))?;
    let pr_number = pr
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;
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
            super::external_id::pr_event(action, pr_number, None),
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
    let status = github_external_status(
        state,
        merged,
        draft,
        pr.get("title").and_then(serde_json::Value::as_str),
        pr.get("html_url").and_then(serde_json::Value::as_str),
        pr.pointer("/base/ref").and_then(serde_json::Value::as_str),
        pr.pointer("/head/ref").and_then(serde_json::Value::as_str),
    );
    Some(WebhookStatusObservation::PullRequest {
        subject_key: GithubPrSubject::new(context.owner, context.repo, pr_number).key(),
        status,
    })
}

async fn normalize_pull_request_sync(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
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
            super::external_id::pr_event("synchronize", pr_number, sync_sha),
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
        .ok_or_else(|| AdapterError::BadRequest("Missing issue number".into()))?;
    let comment = payload
        .get("comment")
        .ok_or_else(|| AdapterError::BadRequest("Missing comment in payload".into()))?;
    let body = comment
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let comment_id = comment
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let source_task_id =
        crate::adapters::footer::parse_task_id_from_body(context.frontend_url, body);
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
        .ok_or_else(|| AdapterError::BadRequest("Missing PR number".into()))?;
    let comment = payload
        .get("comment")
        .ok_or_else(|| AdapterError::BadRequest("Missing comment in payload".into()))?;
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
    message.source_task_id =
        crate::adapters::footer::parse_task_id_from_body(context.frontend_url, body);
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

    if let Some(decision) =
        ReviewDecision::from_review_state(review_state).filter(|decision| decision.is_verdict())
    {
        observations.push(WebhookStatusObservation::ReviewDecision {
            subject_key: GithubPrSubject::new(context.owner, context.repo, pr_number).key(),
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

const CHECK_RUN_FAILURE_CONCLUSIONS: &[&str] =
    &["failure", "timed_out", "cancelled", "action_required"];
const CHECK_RUN_SUMMARY_MAX_LEN: usize = 2000;

async fn normalize_check_run(
    payload: &serde_json::Value,
    context: &NormalizeContext<'_>,
) -> Result<PrimaryWebhook, AdapterError> {
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("completed") {
        return Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction));
    }
    let check_run = payload
        .get("check_run")
        .ok_or_else(|| AdapterError::BadRequest("Missing check_run in payload".into()))?;
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
            if output_summary.len() > CHECK_RUN_SUMMARY_MAX_LEN {
                let truncated = &output_summary[..CHECK_RUN_SUMMARY_MAX_LEN];
                let _ = write!(text, "{truncated}...");
            } else {
                let _ = write!(text, "{output_summary}");
            }
        }
    }
    Ok(PrimaryWebhook::Message(Box::new(
        inbound_message(
            context,
            pr_number,
            Some(pr_number.to_string()),
            text,
            Audience::SubjectProducer,
            super::external_id::check_run(check_run_id),
        )
        .await,
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
        .ok_or_else(|| AdapterError::BadRequest("Missing check_suite in payload".into()))?;
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
    let user_id =
        super::auth::resolve_sender_user(context.sender_login, context.rule, context.db).await;
    InboundMessage {
        adapter_kind: AdapterKind::GithubPr,
        source_id: context.rule.source_matcher.clone(),
        thread_id,
        message_thread_id: pr_number.to_string(),
        user_id,
        author: crate::adapters::models::author_handle(context.sender_login),
        text,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": context.owner,
            "repo": context.repo,
            "pr_number": pr_number,
        }),
        adapter_context: serde_json::json!({}),
        external_event_id: Some(external_event_id),
        source_task_id: None,
        audience,
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

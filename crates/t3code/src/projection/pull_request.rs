//! The pull requests bound to a conversation.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use crate::dao::{Conversation, ConversationPullRequest};
use crate::ports::{is_draft, t3_pr_state};

/// Every pull request a Task is bound to, as T3's `ThreadPullRequestLink`,
/// primary first — see [`Conversation::pull_requests`] for that order.
///
/// `linkedPullRequest` beside it is the legacy single reference, and stays the
/// first element of this list.
pub(super) fn thread_pull_requests(conversation: &Conversation) -> Vec<Value> {
    let Some(bound) = conversation.pull_requests.as_deref() else {
        return Vec::new();
    };
    bound
        .iter()
        .filter_map(|pr| {
            let repository = non_empty(&pr.repository)?;
            let url = non_empty(&pr.url)?;
            if pr.number <= 0 {
                return None;
            }
            // No `watch`: every active Binding routes its events to the
            // agent regardless of this Task's work, so reporting one here
            // would read as T3's "stay in Working while watching a pull
            // request" and mark the thread Waiting for as long as it stays
            // linked, even with no work pending.
            Some(json!({
                "host": pull_request_host(&url),
                "repository": repository,
                "number": pr.number,
                "url": url,
                "source": thread_pull_request_source(&pr.origin, pr.bind_reason.as_deref()),
                "linkedAt": instant(pr.linked_at),
                "snapshot": pull_request_snapshot(pr, &repository),
                // Moatless reads no stack from GitHub, so a link never carries
                // one and the client chains by base and head branch instead.
                "stack": Value::Null
            }))
        })
        .collect()
}

/// The legacy single reference for the primary of [`thread_pull_requests`].
///
/// Placed against the same `projectId` the row carries; `ThreadPullRequestKey`
/// is host-level and carries no project, which is why this is built here rather
/// than sliced off the link.
pub(super) fn legacy_linked_pull_request(link: Option<&Value>, project_id: &str) -> Value {
    let Some(link) = link else {
        return Value::Null;
    };
    json!({
        "projectId": project_id,
        "repository": link["repository"],
        "number": link["number"],
        "url": link["url"]
    })
}

/// The git host a pull request URL names, or `github.com` where it names none.
///
/// T3 keys a link by host so the same pull request reached through two projects
/// compares equal; every binding here comes from the GitHub adapter, hence that
/// default.
fn pull_request_host(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split('/')
        .next()
        .and_then(non_empty)
        .unwrap_or_else(|| "github.com".to_string())
}

/// Who linked a pull request, as T3 names it.
///
/// Only a Binding whose Task filed or created the subject maps onto one of
/// T3's authors; everything else reports `manual`.
fn thread_pull_request_source(origin: &str, bind_reason: Option<&str>) -> &'static str {
    // `TASK_OWNED_BINDING_ORIGIN` and `BindReason::Creator` from
    // `moatless_adapter_model::binding`, spelled out: that crate sits behind a
    // band this one reaches through a seam.
    match (origin, bind_reason) {
        ("task_owned", _) | (_, Some("creator")) => "created",
        _ => "manual",
    }
}

/// The host state a fetched binding carries, or `Null` while its blob is still
/// NULL, leaving the badge pending until then.
///
/// Gate and stand-ins match `bound_pull_request_from_binding`
/// (`crates/server/src/t3code/backend.rs`), which answers
/// `pullRequests.summary` off the same blob; the two readers must not disagree.
fn pull_request_snapshot(pr: &ConversationPullRequest, repository: &str) -> Value {
    // Only a refresh writes the branches, which is what makes them the gate.
    let Some(head_branch) = pr.head_ref.as_deref().and_then(non_empty) else {
        return Value::Null;
    };
    let Some(base_branch) = pr.base_ref.as_deref().and_then(non_empty) else {
        return Value::Null;
    };
    // Trimmed, not merely non-empty: the contract types `title` as a trimmed
    // non-empty string, and a blank one fails validation.
    let title = pr
        .title
        .as_deref()
        .and_then(non_empty)
        .unwrap_or_else(|| format!("{repository}#{}", pr.number));
    let state_kind = pr.state_kind.as_deref().unwrap_or("open");
    let mut snapshot = Map::from_iter([
        ("state".to_string(), json!(t3_pr_state(state_kind))),
        ("title".to_string(), json!(title)),
        ("headBranch".to_string(), json!(head_branch)),
        ("baseBranch".to_string(), json!(base_branch)),
        ("isDraft".to_string(), json!(is_draft(state_kind))),
        // GitHub's own stamp, never `status_read_at`: reporting the read as
        // the change would claim the pull request moved on every refresh.
        (
            "updatedAt".to_string(),
            nullable_instant(pr.updated_at.as_deref().and_then(github_instant)),
        ),
        ("syncedAt".to_string(), json!(instant(pr.status_read_at))),
    ]);
    if let Some(decision) = pr.review_decision.as_deref().and_then(t3_review_decision) {
        snapshot.insert("reviewDecision".to_string(), json!(decision));
    }
    if let Some(state) = pr.mergeable_state.as_deref() {
        snapshot.insert("mergeability".to_string(), json!(t3_mergeability(state)));
    }
    if let Some(state) = pr.checks_state.as_deref().and_then(t3_checks_state) {
        snapshot.insert("checksState".to_string(), json!(state));
    }
    Value::Object(snapshot)
}

/// A checks rollup the blob stores, if it is one of T3's
/// `PullRequestChecksState` values; anything else fails the contract.
fn t3_checks_state(state: &str) -> Option<&'static str> {
    ["passing", "failing", "pending"]
        .into_iter()
        .find(|known| *known == state)
}

/// A GitHub ISO 8601 timestamp, or `None` for one that does not parse, which
/// the contract's `IsoDateTime` would refuse.
fn github_instant(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
}

/// GitHub's review verdict as T3's `PullRequestReviewDecision`, or `None` for a
/// verdict T3 has no word for. `dismissed` and `commented` are the two: neither
/// approves nor blocks a merge, so both read as no decision at all.
fn t3_review_decision(decision: &str) -> Option<&'static str> {
    match decision {
        "approved" => Some("approved"),
        "changes_requested" => Some("changes-requested"),
        "review_required" => Some("review-required"),
        _ => None,
    }
}

/// GitHub's mergeable state as T3's `PullRequestMergeability`. Only `dirty`
/// means merge conflicts; `blocked`, `behind` and `unstable` describe a
/// mergeable branch waiting on something else, and T3 has no word for that.
fn t3_mergeability(state: &str) -> &'static str {
    match state {
        "dirty" => "conflicting",
        "clean" => "mergeable",
        _ => "unknown",
    }
}

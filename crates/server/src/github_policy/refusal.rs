//! Why a request falls outside the policy, and the review-event rule both
//! the REST and GraphQL checks share.

use moatless_core::workspace_identity_grants::GitHubCapability;

/// Why a request falls outside the policy. `rule` is the stable name the log
/// line carries; `message` is returned to the caller under `enforce`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub rule: &'static str,
    pub message: String,
}

impl Refusal {
    pub(crate) fn new(rule: &'static str, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
        }
    }
}

/// The `rule` a refusal for lacking `capability` logs.
pub(super) fn capability_rule(capability: GitHubCapability) -> &'static str {
    match capability {
        GitHubCapability::Approve => "approve",
        GitHubCapability::Merge => "merge",
        GitHubCapability::PushTags => "push_tags",
        GitHubCapability::PushDefaultBranch => "push_default_branch",
    }
}

/// Whether a review's `event` approves. An event that cannot be read counts as
/// `APPROVE`; a missing one opens a pending review, which approves nothing.
pub(super) fn review_event_approves(event: Option<&serde_json::Value>) -> bool {
    match event {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::String(event)) => event.eq_ignore_ascii_case("APPROVE"),
        Some(_) => true,
    }
}

/// [`review_event_approves`] for a REST review body.
pub(super) fn review_needs_approve(body: Option<&[u8]>) -> bool {
    body.and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
        .and_then(|value| {
            value
                .as_object()
                .map(|o| review_event_approves(o.get("event")))
        })
        .unwrap_or(true)
}

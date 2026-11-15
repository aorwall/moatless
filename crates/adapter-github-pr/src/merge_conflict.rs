//! Merge-conflict candidate derivation and durable execution.

use moatless_workspace_types::repository::GitHostRepo;
use tracing::{debug, info, warn};

use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::SubscriptionMatch;

use super::router::GithubPrInboundRouter;
use super::signals::MergeableState;
use super::subject::GithubPrSubject;
use super::text::merge_conflict_text;

/// A PR-bearing event that should schedule a durable conflict check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) pr_number: u64,
}

/// Facts persisted for one durable merge-conflict execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Execution {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) pr_number: u64,
    pub(super) sender_login: String,
}

/// Derive a merge-conflict candidate from an event without I/O.
///
/// No `check_run` candidate: its suite's completion rechecks the same head
/// once per suite rather than once per run, and the last suite to finish still
/// records the post-CI `mergeable_state`.
pub(super) fn candidate(event_type: &str, payload: &serde_json::Value) -> Option<Candidate> {
    let pr_number = match event_type {
        "pull_request" => {
            if payload.get("action").and_then(serde_json::Value::as_str) == Some("closed") {
                return None;
            }
            payload
                .pointer("/pull_request/number")
                .and_then(serde_json::Value::as_u64)
        }
        "pull_request_review" | "pull_request_review_comment" => payload
            .pointer("/pull_request/number")
            .and_then(serde_json::Value::as_u64),
        "issue_comment" => {
            payload.pointer("/issue/pull_request")?;
            payload
                .pointer("/issue/number")
                .and_then(serde_json::Value::as_u64)
        }
        "check_suite" => {
            if payload.get("action").and_then(serde_json::Value::as_str) != Some("completed") {
                return None;
            }
            payload
                .pointer("/check_suite/pull_requests/0/number")
                .and_then(serde_json::Value::as_u64)
        }
        _ => None,
    }?;

    Some(Candidate { pr_number })
}

impl GithubPrInboundRouter {
    /// Execute one durable merge-conflict check.
    ///
    /// Skips before auth/GitHub access when no active Binding tracks the PR;
    /// once active, failures that prevent the check are retryable, while
    /// persistence and notification stay best-effort.
    pub(super) async fn run_merge_conflict(
        &self,
        connection: &AdapterConnection,
        rule: &SubscriptionMatch,
        filter: &crate::config::FilterKey,
        execution: &Execution,
    ) -> Result<(), AdapterError> {
        let subject = GithubPrSubject::new(&execution.owner, &execution.repo, execution.pr_number);
        let subject_key = subject.key();

        if !self
            .task_bindings
            .for_subject("github", "github_pr", &subject_key)
            .await?
            .iter()
            .any(|binding| binding.active)
        {
            debug!(
                subject_key,
                "No active bindings; skipping merge-conflict check"
            );
            return Ok(());
        }

        let Some(adapter) = self.adapter.as_ref() else {
            return Ok(());
        };

        let authenticated = adapter
            .auth()
            .client_for_durable_work(
                &execution.sender_login,
                rule,
                self.users.as_ref(),
                &GitHostRepo::github(&execution.owner, &execution.repo),
            )
            .await?;

        let pr = authenticated
            .api
            .get_pr_mergeable(&execution.owner, &execution.repo, execution.pr_number)
            .await?;

        let mergeable = pr
            .mergeable_state
            .as_deref()
            .map(MergeableState::from_github);
        let Some(mergeable) = mergeable.filter(|mergeable| mergeable.is_settled()) else {
            debug!(
                subject_key,
                mergeable_state = ?pr.mergeable_state,
                "Mergeable state unresolved; skipping persist + notify"
            );
            return Ok(());
        };

        let pr_is_open = pr.state.eq_ignore_ascii_case("open");
        let conflict_message = InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: Some(execution.pr_number.to_string()),
            message_thread_id: execution.pr_number.to_string(),
            user_id: authenticated.user_id,
            author: None,
            sender_external_id: None,
            requester: None,
            text: merge_conflict_text(
                &execution.owner,
                &execution.repo,
                execution.pr_number,
                pr.html_url.as_deref().unwrap_or(""),
            ),
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({
                "owner": execution.owner,
                "repo": execution.repo,
                "pr_number": execution.pr_number,
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::merge_conflict(
                &GithubPrSubject::new(&execution.owner, &execution.repo, execution.pr_number),
                &pr.head.sha,
            )),
            source_task_id: None,
            audience: Audience::SubjectProducer,
            replies_to: Vec::new(),
            event: Some(super::config::inbound_event(super::config::MERGE_CONFLICT)),
        };

        if !self.record_state(&subject_key, mergeable, pr_is_open).await {
            return Ok(());
        }
        info!(subject_key, "Delivering merge-conflict notification");
        if let Err(error) = self
            .route_event(connection, Some(rule), filter, conflict_message)
            .await
        {
            warn!(
                subject_key,
                %error,
                "Failed to deliver merge-conflict notification"
            );
        }
        Ok(())
    }

    /// Persist mergeability, and say whether this is an open PR's transition
    /// to conflict, the only state worth a notice.
    ///
    /// Both are deliberately best-effort: the durable execution retries
    /// failures that prevent the check, not an FYI that was already decided.
    async fn record_state(
        &self,
        subject_key: &str,
        mergeable: MergeableState,
        pr_is_open: bool,
    ) -> bool {
        let binding_repo = self.task_bindings.clone();
        let bindings = match binding_repo
            .for_subject("github", "github_pr", subject_key)
            .await
        {
            Ok(bindings) => bindings,
            Err(error) => {
                warn!(
                    subject_key,
                    %error,
                    "Failed to load bindings for mergeable-state update"
                );
                return false;
            }
        };

        let is_news = bindings.iter().any(|binding| {
            binding.active && binding.external_mergeable_state.as_deref() != Some("dirty")
        });

        for binding in &bindings {
            if let Err(error) = binding_repo
                .record_mergeable_state(&binding.id, Some(mergeable.as_str()))
                .await
            {
                warn!(
                    binding_id = %binding.id,
                    %error,
                    "Failed to persist mergeable state"
                );
            }
        }

        pr_is_open && mergeable.is_conflicted() && is_news
    }
}

#[cfg(test)]
mod tests {
    use super::candidate;

    #[test]
    fn candidates_are_derived_per_event_shape() {
        let cases = [
            (
                "pull_request",
                serde_json::json!({"action": "synchronize", "pull_request": {"number": 12}}),
                Some(12),
            ),
            (
                "pull_request",
                serde_json::json!({"action": "closed", "pull_request": {"number": 12}}),
                None,
            ),
            (
                "pull_request_review",
                serde_json::json!({"pull_request": {"number": 34}}),
                Some(34),
            ),
            (
                "pull_request_review_comment",
                serde_json::json!({"pull_request": {"number": 35}}),
                Some(35),
            ),
            (
                "issue_comment",
                serde_json::json!({"issue": {"number": 5, "pull_request": {"url": "x"}}}),
                Some(5),
            ),
            (
                "issue_comment",
                serde_json::json!({"issue": {"number": 5}}),
                None,
            ),
            (
                "check_run",
                serde_json::json!({"action": "completed", "check_run": {"pull_requests": [{"number": 9}]}}),
                None,
            ),
            (
                "check_suite",
                serde_json::json!({"action": "completed", "check_suite": {"pull_requests": [{"number": 10}]}}),
                Some(10),
            ),
            (
                "check_suite",
                serde_json::json!({"action": "requested", "check_suite": {"pull_requests": [{"number": 10}]}}),
                None,
            ),
            ("push", serde_json::json!({}), None),
        ];

        for (event_type, payload, expected) in cases {
            assert_eq!(
                candidate(event_type, &payload).map(|candidate| candidate.pr_number),
                expected,
                "{event_type}"
            );
        }
    }
}

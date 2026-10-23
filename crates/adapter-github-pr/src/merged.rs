//! Merged-PR notification: candidate derivation and durable execution.
//!
//! Tells tasks still working a pull request that it landed. GitHub has no
//! `merged` action, so this derives from `pull_request` `closed` with
//! `merged: true`, gated by its own name ([`super::config::PR_MERGED`])
//! rather than the carrier's.
//!
//! Recipients are the Bindings auto-close does not own: auto-close winds its
//! own tasks down and appends its own notice ([`super::routing`]); this
//! notifies everyone else, so the two kinds never both deliver to one Task.

use std::fmt::Write;

use tracing::{debug, info, warn};

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::error::AdapterError;
use moatless_loops::adapters::subject_events::{SubjectEvent, route_subject_event};
use moatless_ports::task::{CloseReason, TaskStatus};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::SubscriptionMatch;
use moatless_workspace_types::repository::GITHUB_HOST;

use super::router::GithubPrInboundRouter;
use super::routing::{ADAPTER, ADAPTER_LABEL, SUBJECT_KIND};
use super::subject::GithubPrSubject;

/// The merged pull request, as parsed from the webhook that announced it.
///
/// Serializable: carried as the payload of a `github.pr_merged` Adapter
/// delivery record (see [`super::delivery`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct MergedPr {
    pub(super) pr_number: u64,
    pub(super) title: String,
    pub(super) html_url: String,
    /// Login GitHub names as having merged it, falling back to the webhook
    /// sender when `merged_by` is absent.
    pub(super) merged_by: String,
    #[serde(default)]
    pub(super) pr_author_login: Option<String>,
    pub(super) base_ref: String,
    pub(super) head_ref: String,
    pub(super) merge_commit_sha: String,
}

/// Facts persisted for one durable merged-PR notification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Execution {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) merged: MergedPr,
}

/// Derive a merged-PR candidate from an event without I/O.
///
/// `closed` with `merged: false` is a pull request abandoned rather than
/// landed, and produces no candidate.
pub(super) fn candidate(event_type: &str, payload: &serde_json::Value) -> Option<MergedPr> {
    if event_type != "pull_request"
        || payload.get("action").and_then(serde_json::Value::as_str) != Some("closed")
    {
        return None;
    }
    let pr = payload.get("pull_request")?;
    if pr.get("merged").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let pr_number = pr.get("number").and_then(serde_json::Value::as_u64)?;
    let merged_by = pr
        .pointer("/merged_by/login")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            payload
                .pointer("/sender/login")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("")
        .to_string();

    Some(MergedPr {
        pr_number,
        title: string_at(pr, "/title"),
        html_url: string_at(pr, "/html_url"),
        merged_by,
        pr_author_login: super::webhook::pr_author_login(payload).map(str::to_string),
        base_ref: string_at(pr, "/base/ref"),
        head_ref: string_at(pr, "/head/ref"),
        merge_commit_sha: string_at(pr, "/merge_commit_sha"),
    })
}

fn string_at(value: &serde_json::Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Render the merged-PR notification body.
pub(super) fn merged_text(merged: &MergedPr, owner: &str, repo: &str) -> String {
    let mut text = format!(
        "## ✅ Pull Request Merged: {owner}/{repo}#{pr_number}\n\n\
         **Title:** {title}\n",
        pr_number = merged.pr_number,
        title = merged.title,
    );
    if !merged.merged_by.is_empty() {
        let _ = writeln!(text, "**Merged by:** @{}", merged.merged_by);
    }
    if !merged.head_ref.is_empty() && !merged.base_ref.is_empty() {
        let _ = writeln!(
            text,
            "**Branch:** {} → {}",
            merged.head_ref, merged.base_ref
        );
    }
    if !merged.merge_commit_sha.is_empty() {
        let short = &merged.merge_commit_sha[..7.min(merged.merge_commit_sha.len())];
        let _ = writeln!(text, "**Merge commit:** `{short}`");
    }
    if !merged.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", merged.html_url);
    }
    text
}

/// True when the trigger's auto-close owns this Binding's Task.
///
/// Auto-close reaches exactly the Tasks its own trigger created
/// ([`GithubPrInboundRouter::close_subject`]) and delivers its close notice to
/// each itself.
fn auto_close_owns(rule: &SubscriptionMatch, binding: &TaskBinding) -> bool {
    rule.close_task_on_close
        && binding.origin == ADAPTER_EVENT_BINDING_ORIGIN
        && binding.loop_id.as_deref() == Some(rule.id.as_str())
}

impl GithubPrInboundRouter {
    /// Execute one durable merged-PR notification.
    ///
    /// Needs no GitHub credential (everything is already in the admitting
    /// webhook). A pull request with no Task left to tell is a skip, not a
    /// failure; a failed delivery retries, deduped by the per-Binding receipt
    /// keyed on [`super::external_id::pr_merged`].
    pub(super) async fn run_pr_merged(
        &self,
        connection: &AdapterConnection,
        rule: &SubscriptionMatch,
        execution: &Execution,
    ) -> Result<(), AdapterError> {
        let merged = &execution.merged;
        self.close_review_guide_tasks(execution).await;
        let subject_key =
            GithubPrSubject::new(&execution.owner, &execution.repo, merged.pr_number).key();

        let recipients: Vec<TaskBinding> = self
            .task_bindings
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
            .await?
            .into_iter()
            .filter(|binding| binding.active && !auto_close_owns(rule, binding))
            .collect();
        if recipients.is_empty() {
            debug!(
                subject_key,
                "No task to tell about the merge; skipping merged-PR notice"
            );
            return Ok(());
        }

        let notice = InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: Some(merged.pr_number.to_string()),
            message_thread_id: merged.pr_number.to_string(),
            user_id: super::auth::resolve_sender_user(
                &merged.merged_by,
                merged.pr_author_login.as_deref(),
                rule,
                self.users.as_ref(),
            )
            .await,
            author: moatless_types::adapter::author_handle(&merged.merged_by),
            sender_external_id: None,
            text: merged_text(merged, &execution.owner, &execution.repo),
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": execution.owner,
                "repo": execution.repo,
                "pr_number": merged.pr_number,
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::pr_merged(&GithubPrSubject::new(
                &execution.owner,
                &execution.repo,
                merged.pr_number,
            ))),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
        };

        info!(
            subject_key,
            recipients = recipients.len(),
            "Delivering merged-PR notification"
        );
        route_subject_event(
            &self.routing,
            connection,
            rule,
            &notice,
            SubjectEvent {
                adapter_label: ADAPTER_LABEL,
                binding_provider: ADAPTER,
                subject_kind: SUBJECT_KIND,
                subject_key: &subject_key,
                bindings: recipients,
                author_task_id: None,
                new_task_binding_id: None,
            },
        )
        .await?;
        Ok(())
    }
}

impl GithubPrInboundRouter {
    /// Close the Tasks that wrote the merged pull request's review guide. A
    /// failure is logged rather than returned: returning it would retry the
    /// whole delivery for a close that has no notice to resend.
    pub(super) async fn close_review_guide_tasks(&self, execution: &Execution) {
        let repository = format!("{}/{}", execution.owner, execution.repo);
        let number = execution.merged.pr_number;
        let tasks = match self
            .review_guides
            .guide_tasks(GITHUB_HOST, &repository, number)
            .await
        {
            Ok(tasks) => tasks,
            Err(error) => {
                warn!(repository, number, %error, "Failed to read the review guide's tasks");
                return;
            }
        };
        #[allow(clippy::disallowed_methods)]
        // reason: a webhook delivery carries no caller identity.
        let sys = ScopeContext::system();
        for task_id in tasks {
            match self.tasks.describe(&task_id, &sys).await {
                Ok(Some(task)) if task.status != TaskStatus::Closed => {}
                Ok(_) => continue,
                Err(error) => {
                    warn!(task_id, %error, "Failed to read a review guide task");
                    continue;
                }
            }
            let reason = CloseReason {
                actor: "github".to_string(),
                payload: serde_json::json!({ "reason": "pr_merged", "pr_number": number }),
            };
            match self.task_closer.close(&task_id, reason).await {
                Ok(()) => info!(
                    task_id,
                    repository, number, "Review guide task closed on merge"
                ),
                Err(error) => warn!(task_id, %error, "Failed to close a review guide task"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterKind;
    use moatless_types::loops::RoutingMode;

    use super::*;

    fn closed_payload(merged: bool, merged_by: Option<&str>) -> serde_json::Value {
        let mut pr = serde_json::json!({
            "number": 42,
            "title": "feat: land it",
            "html_url": "https://github.com/soaplabs/repo/pull/42",
            "merged": merged,
            "merge_commit_sha": "abcdef0123456789",
            "base": {"ref": "main"},
            "head": {"ref": "feature"},
        });
        if let Some(login) = merged_by {
            pr["merged_by"] = serde_json::json!({"login": login});
        }
        serde_json::json!({
            "action": "closed",
            "sender": {"login": "webhook-sender"},
            "pull_request": pr,
        })
    }

    fn rule(id: &str, close_task_on_close: bool) -> SubscriptionMatch {
        SubscriptionMatch {
            name: "test-loop".to_string(),
            id: id.into(),
            adapter_kind: AdapterKind::Github,
            connection_id: "pc_test".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            workspace_id: "ws-1".into(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            memory_preload: Vec::new(),
            skills: Vec::new(),
            run_as_user_id: Some("alice".into()),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            node_selector: None,
            stop_when_done: None,
            message_buffer_seconds: None,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            close_task_on_close,
            notify_on_success: false,
            active: true,
            created_by: "alice".into(),
            scope: Scope::User,
            owner_user_id: Some("alice".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn binding(origin: &str, loop_id: Option<&str>) -> TaskBinding {
        TaskBinding {
            id: "tb_1".into(),
            task_id: "task-1".into(),
            adapter_kind: AdapterKind::GithubPr,
            subject_kind: SUBJECT_KIND.into(),
            subject_key: "soaplabs/repo#42".into(),
            connection_id: "pc_test".into(),
            loop_id: loop_id.map(str::to_string),
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: origin.into(),
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
            watching: true,
        }
    }

    #[test]
    fn only_a_landed_pull_request_is_a_candidate() {
        let merged = candidate("pull_request", &closed_payload(true, Some("alice")))
            .expect("a merged close is a candidate");
        assert_eq!(merged.pr_number, 42);
        assert_eq!(merged.merged_by, "alice");
        assert_eq!(merged.base_ref, "main");
        assert_eq!(merged.head_ref, "feature");
        assert_eq!(merged.merge_commit_sha, "abcdef0123456789");

        // Abandoned, not landed.
        assert!(candidate("pull_request", &closed_payload(false, Some("alice"))).is_none());
        // Every other action and event on the same payload shape.
        for (event_type, action) in [
            ("pull_request", "opened"),
            ("pull_request", "synchronize"),
            ("pull_request_review", "closed"),
            ("issue_comment", "closed"),
        ] {
            let mut payload = closed_payload(true, Some("alice"));
            payload["action"] = serde_json::json!(action);
            assert!(
                candidate(event_type, &payload).is_none(),
                "{event_type}/{action}"
            );
        }
    }

    #[test]
    fn the_webhook_sender_stands_in_for_an_absent_merged_by() {
        let merged =
            candidate("pull_request", &closed_payload(true, None)).expect("still a candidate");
        assert_eq!(merged.merged_by, "webhook-sender");
    }

    #[test]
    fn merged_text_names_the_pull_request_and_omits_what_the_payload_lacks() {
        let merged = candidate("pull_request", &closed_payload(true, Some("alice")))
            .expect("a merged close is a candidate");
        let text = merged_text(&merged, "soaplabs", "repo");
        assert!(text.contains("## ✅ Pull Request Merged: soaplabs/repo#42"));
        assert!(text.contains("**Title:** feat: land it"));
        assert!(text.contains("**Merged by:** @alice"));
        assert!(text.contains("**Branch:** feature → main"));
        assert!(text.contains("**Merge commit:** `abcdef0`"));
        assert!(text.contains("**Link:** https://github.com/soaplabs/repo/pull/42"));

        let bare = MergedPr {
            html_url: String::new(),
            merged_by: String::new(),
            base_ref: String::new(),
            head_ref: String::new(),
            merge_commit_sha: String::new(),
            ..merged
        };
        let text = merged_text(&bare, "soaplabs", "repo");
        assert!(text.contains("## ✅ Pull Request Merged: soaplabs/repo#42"));
        for absent in [
            "**Merged by:**",
            "**Branch:**",
            "**Merge commit:**",
            "**Link:**",
        ] {
            assert!(!text.contains(absent), "{absent}");
        }
    }

    /// The recipient split is what keeps this kind and `github.close`
    /// independent of the order they run in, so it is asserted on the predicate
    /// rather than left to the two handlers to agree on at runtime.
    #[test]
    fn auto_close_owns_only_the_tasks_its_own_trigger_created() {
        let closing = rule("ir_1", true);
        assert!(auto_close_owns(
            &closing,
            &binding(ADAPTER_EVENT_BINDING_ORIGIN, Some("ir_1"))
        ));
        // A task that connected itself to the pull request, e.g. by opening it.
        assert!(!auto_close_owns(
            &closing,
            &binding("connected_thread", Some("ir_1"))
        ));
        // Another trigger's task; this one is not closing it.
        assert!(!auto_close_owns(
            &closing,
            &binding(ADAPTER_EVENT_BINDING_ORIGIN, Some("ir_2"))
        ));
        // A trigger that leaves its tasks open closes nothing, so it owns none
        // of them and every binding hears about the merge.
        let keeping = rule("ir_1", false);
        assert!(!auto_close_owns(
            &keeping,
            &binding(ADAPTER_EVENT_BINDING_ORIGIN, Some("ir_1"))
        ));
    }

    mod review_guide_close {
        use moatless_ports::task::{
            MockMessageAcceptor, MockTaskFinisher, MockTaskReader, TaskDescriptor, TaskStatus,
        };

        use super::super::{Execution, MergedPr};
        use crate::router::MockReviewGuideTasks;
        use crate::routing::tests::notify_and_maybe_close_tests::router_with;

        fn task(task_id: &str, status: TaskStatus) -> TaskDescriptor {
            TaskDescriptor {
                task_id: task_id.to_string(),
                status,
                name: None,
                owner_user_id: None,
                workspace_id: None,
                created_by: None,
                ttl_minutes: 0,
            }
        }

        #[tokio::test]
        async fn a_merge_closes_the_open_review_guide_task_only() {
            let mut review_guides = MockReviewGuideTasks::new();
            review_guides
                .expect_guide_tasks()
                .withf(|host, repository, number| {
                    host == "github.com" && repository == "soaplabs/repo" && *number == 42
                })
                .returning(|_, _, _| Ok(vec!["t_open".into(), "t_closed".into()]));
            let mut tasks = MockTaskReader::new();
            tasks.expect_describe().returning(|task_id, _| {
                Ok(Some(if task_id == "t_open" {
                    task(task_id, TaskStatus::Open)
                } else {
                    task(task_id, TaskStatus::Closed)
                }))
            });
            let mut task_closer = MockTaskFinisher::new();
            task_closer
                .expect_close()
                .withf(|task_id, reason| {
                    task_id == "t_open" && reason.payload["reason"] == "pr_merged"
                })
                .times(1)
                .returning(|_, _| Ok(()));

            let router = router_with(
                MockMessageAcceptor::new(),
                task_closer,
                tasks,
                review_guides,
            );
            router
                .close_review_guide_tasks(&Execution {
                    owner: "soaplabs".into(),
                    repo: "repo".into(),
                    merged: MergedPr {
                        pr_number: 42,
                        title: "feat: land it".into(),
                        html_url: String::new(),
                        merged_by: "octocat".into(),
                        pr_author_login: None,
                        base_ref: "main".into(),
                        head_ref: "feature".into(),
                        merge_commit_sha: String::new(),
                    },
                })
                .await;
        }
    }
}

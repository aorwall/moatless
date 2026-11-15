//! Post-merge workflow-run parsing and delivery for the GitHub PR adapter.

use moatless_workspace_types::repository::GitHostRepo;
use std::fmt::Write;

use tracing::{debug, info, warn};

use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::SubscriptionMatch;

use super::api::WorkflowRunSummary;
use super::router::GithubPrInboundRouter;
use super::subject::GithubPrSubject;

/// Parsed fields of a base-branch `workflow_run.completed` event.
///
/// Serializable: carried as the payload of a `github.post_merge` Adapter
/// delivery record (see [`super::delivery`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PostMergeRun {
    pub(super) run_id: u64,
    pub(super) name: String,
    pub(super) conclusion: String,
    pub(super) head_sha: String,
    pub(super) head_branch: String,
    pub(super) html_url: String,
}

/// Facts persisted for one durable post-merge execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Execution {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) run: PostMergeRun,
    pub(super) sender_login: String,
}

/// Conclusions worth notifying a merged PR's tasks about. `cancelled` is
/// deliberately absent for the same reason it is in [`super::webhook`]: a run
/// cancelled by a concurrency group or by hand reports nothing about the merge.
const NOTIFIABLE_CONCLUSIONS: &[&str] = &[
    "success",
    "failure",
    "timed_out",
    "action_required",
    "startup_failure",
];

/// Derive a post-merge candidate from an event without I/O.
pub(super) fn candidate(event_type: &str, payload: &serde_json::Value) -> Option<PostMergeRun> {
    if event_type != "workflow_run"
        || payload.get("action").and_then(serde_json::Value::as_str) != Some("completed")
    {
        return None;
    }
    let run = payload.get("workflow_run")?;
    if run.get("event").and_then(serde_json::Value::as_str) != Some("push") {
        return None;
    }
    if run
        .get("pull_requests")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|pull_requests| !pull_requests.is_empty())
    {
        return None;
    }
    let conclusion = run
        .get("conclusion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !NOTIFIABLE_CONCLUSIONS.contains(&conclusion) {
        return None;
    }
    let head_sha = run
        .get("head_sha")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if head_sha.is_empty() {
        return None;
    }
    Some(PostMergeRun {
        run_id: run
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        name: run
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("workflow")
            .to_string(),
        conclusion: conclusion.to_string(),
        head_sha: head_sha.to_string(),
        head_branch: run
            .get("head_branch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        html_url: run
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Conclusions that leave a pipeline green. `cancelled` is deliberately
/// absent: a cancelled run proves nothing passed.
const PASSING_CONCLUSIONS: &[&str] = &["success", "skipped", "neutral"];

/// The workflow names, when every run a push of the commit started has
/// completed and passed; `None` while one runs or after one failed.
pub(super) fn pipeline_passed(runs: &[WorkflowRunSummary]) -> Option<Vec<String>> {
    let passed = !runs.is_empty()
        && runs.iter().all(|run| {
            run.status == "completed"
                && run
                    .conclusion
                    .as_deref()
                    .is_some_and(|c| PASSING_CONCLUSIONS.contains(&c))
        });
    passed.then(|| {
        let mut names: Vec<String> = runs.iter().map(|run| run.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    })
}

/// Render the notice that a merged PR's whole post-merge pipeline passed.
pub(super) fn pipeline_passed_text(
    run: &PostMergeRun,
    workflows: &[String],
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> String {
    let short_sha = &run.head_sha[..7.min(run.head_sha.len())];
    let mut text = format!(
        "## ✅ Post-merge pipeline passed\n\n\
         Every workflow the `{branch}` branch push ran for merged PR \
         {owner}/{repo}#{pr_number} passed.\n\
         **Commit:** `{short_sha}`\n\
         **Workflows:** {workflows}\n",
        branch = run.head_branch,
        workflows = workflows.join(", "),
    );
    if !run.html_url.is_empty() {
        let _ = writeln!(text, "**Last run:** {}", run.html_url);
    }
    text
}

/// Render the post-merge CI notification body.
pub(super) fn post_merge_ci_text(
    run: &PostMergeRun,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> String {
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

impl GithubPrInboundRouter {
    /// Notify a merged PR's tasks of a base-branch run: a failure at once, a
    /// pass only once [`pipeline_passed`] holds for the commit.
    ///
    /// Skips (`Ok(())`) off the default branch, without a merged PR or active
    /// Binding, or mid-pipeline; any other failure retries.
    pub(super) async fn run_post_merge(
        &self,
        connection: &AdapterConnection,
        rule: &SubscriptionMatch,
        filter: &crate::config::FilterKey,
        execution: &Execution,
    ) -> Result<(), AdapterError> {
        let Some(adapter) = self.adapter.as_ref() else {
            return Ok(());
        };

        // Resolve the acting user + token the same way CI/merge-conflict paths do.
        let authenticated = adapter
            .auth()
            .client_for_durable_work(
                &execution.sender_login,
                rule,
                self.users.as_ref(),
                &GitHostRepo::github(&execution.owner, &execution.repo),
            )
            .await?;
        let api = authenticated.api;

        // Confirm the run is on the repo's default (base) branch — post-merge runs
        // live there; runs on other branches are not merge results.
        let default_branch = api
            .get_repo_default_branch(&execution.owner, &execution.repo)
            .await?;
        if default_branch != execution.run.head_branch {
            debug!(
                head_branch = %execution.run.head_branch,
                default_branch,
                "Skipping post-merge CI: run not on default branch"
            );
            return Ok(());
        }

        let pulls = api
            .get_pulls_for_commit(&execution.owner, &execution.repo, &execution.run.head_sha)
            .await?;
        let Some(pr) = pulls.into_iter().find(super::api::CommitPr::is_merged) else {
            debug!(
                sha = %execution.run.head_sha,
                "No merged PR for commit; skipping post-merge CI notice"
            );
            return Ok(());
        };
        let pr_number = pr.number;
        let subject_key = GithubPrSubject::new(&execution.owner, &execution.repo, pr_number).key();

        if !self
            .task_bindings
            .for_subject("github", "github_pr", &subject_key)
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

        let (text, external_event_id) = if execution.run.conclusion == "success" {
            let runs = api
                .list_push_runs_for_commit(
                    &execution.owner,
                    &execution.repo,
                    &execution.run.head_sha,
                )
                .await?;
            let Some(workflows) = pipeline_passed(&runs) else {
                debug!(
                    subject_key,
                    sha = %execution.run.head_sha,
                    "Post-merge pipeline not yet passed; the last run to finish notifies"
                );
                return Ok(());
            };
            (
                pipeline_passed_text(
                    &execution.run,
                    &workflows,
                    &execution.owner,
                    &execution.repo,
                    pr_number,
                ),
                super::external_id::pipeline_passed(
                    &execution.owner,
                    &execution.repo,
                    &execution.run.head_sha,
                ),
            )
        } else {
            (
                post_merge_ci_text(&execution.run, &execution.owner, &execution.repo, pr_number),
                super::external_id::workflow_run(execution.run.run_id),
            )
        };

        let notice = InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: Some(pr_number.to_string()),
            message_thread_id: pr_number.to_string(),
            user_id: authenticated.user_id,
            author: None,
            sender_external_id: None,
            requester: None,
            text,
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({
                "owner": execution.owner,
                "repo": execution.repo,
                "pr_number": pr_number,
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some(external_event_id),
            source_task_id: None,
            audience: Audience::SubjectProducer,
            replies_to: Vec::new(),
            event: Some(super::config::inbound_event(super::config::POST_MERGE)),
        };

        info!(subject_key, "Delivering post-merge CI notification");
        if let Err(error) = self
            .route_event(connection, Some(rule), filter, notice)
            .await
        {
            warn!(
                subject_key,
                %error,
                "Failed to deliver post-merge CI notification"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PostMergeRun, WorkflowRunSummary, candidate, pipeline_passed, pipeline_passed_text,
        post_merge_ci_text,
    };

    fn run(name: &str, status: &str, conclusion: Option<&str>) -> WorkflowRunSummary {
        WorkflowRunSummary {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
        }
    }

    #[test]
    fn the_pipeline_passes_only_once_every_push_run_completed_green() {
        let passed = [
            run("CI", "completed", Some("success")),
            run("Docs", "completed", Some("skipped")),
            run("CI", "completed", Some("success")),
        ];
        assert_eq!(
            pipeline_passed(&passed),
            Some(vec!["CI".to_string(), "Docs".to_string()])
        );

        for unfinished in [
            vec![],
            vec![
                run("CI", "completed", Some("success")),
                run("Deploy", "in_progress", None),
            ],
            vec![
                run("CI", "completed", Some("success")),
                run("Deploy", "completed", Some("failure")),
            ],
            vec![run("CI", "completed", Some("cancelled"))],
        ] {
            assert_eq!(pipeline_passed(&unfinished), None, "{unfinished:?}");
        }
    }

    #[test]
    fn the_pipeline_notice_names_the_commit_and_its_workflows() {
        let last = PostMergeRun {
            run_id: 1,
            name: "Deploy".into(),
            conclusion: "success".into(),
            head_sha: "abcdef012345".into(),
            head_branch: "main".into(),
            html_url: "https://x/run/1".into(),
        };
        let text = pipeline_passed_text(&last, &["CI".into(), "Deploy".into()], "o", "r", 42);
        assert!(text.contains("✅ Post-merge pipeline passed"));
        assert!(text.contains("o/r#42"));
        assert!(text.contains("`abcdef0`"));
        assert!(text.contains("**Workflows:** CI, Deploy"));
    }

    fn workflow_run_payload(
        action: &str,
        event: &str,
        conclusion: &str,
        pull_requests: serde_json::Value,
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
                "pull_requests": pull_requests,
            }
        })
    }

    #[test]
    fn candidates_are_filtered_from_raw_facts_without_external_calls() {
        let accepted = candidate(
            "workflow_run",
            &workflow_run_payload("completed", "push", "success", serde_json::json!([])),
        )
        .expect("completed base-branch push should be a candidate");
        assert_eq!(accepted.run_id, 555);
        assert_eq!(accepted.head_sha, "12b2f924aa30");

        let rejected = [
            (
                "check_run",
                workflow_run_payload("completed", "push", "success", serde_json::json!([])),
            ),
            (
                "workflow_run",
                workflow_run_payload("requested", "push", "success", serde_json::json!([])),
            ),
            (
                "workflow_run",
                workflow_run_payload(
                    "completed",
                    "pull_request",
                    "success",
                    serde_json::json!([]),
                ),
            ),
            (
                "workflow_run",
                workflow_run_payload(
                    "completed",
                    "push",
                    "success",
                    serde_json::json!([{"number": 7}]),
                ),
            ),
            (
                "workflow_run",
                workflow_run_payload("completed", "push", "skipped", serde_json::json!([])),
            ),
        ];
        for (event_type, payload) in rejected {
            assert!(candidate(event_type, &payload).is_none());
        }
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
}

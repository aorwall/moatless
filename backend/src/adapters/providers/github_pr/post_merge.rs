//! Post-merge workflow-run parsing and delivery for the GitHub PR adapter.

use std::fmt::Write;

use tracing::{debug, info, warn};

use crate::adapters::AdapterKind;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, Audience, InboundMessage, SubscriptionMatch};
use crate::bindings::dao::PgTaskBindingDao;
use crate::state::AppState;

use super::subject::GithubPrSubject;

/// Parsed fields of a base-branch `workflow_run.completed` event.
///
/// Serializable because it is carried as the payload of a `github.post_merge`
/// Adapter delivery record (see [`super::delivery`]).
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

const NOTIFIABLE_CONCLUSIONS: &[&str] = &[
    "success",
    "failure",
    "timed_out",
    "cancelled",
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
pub(super) async fn execute(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    execution: &Execution,
) -> Result<(), AdapterError> {
    let Some(adapter) = state.adapters.as_ref().and_then(|a| a.github_pr()) else {
        return Ok(());
    };

    // Resolve the acting user + token the same way CI/merge-conflict paths do.
    let authenticated = adapter
        .auth()
        .client_for_durable_work(&execution.sender_login, rule, &state.db, &execution.owner)
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

    // Map the merge commit back to the PR it merged.
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

    // Skip unless some active task is still connected to the PR.
    if !PgTaskBindingDao(state.db.clone())
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
        adapter_kind: AdapterKind::GithubPr,
        source_id: rule.source_matcher.clone(),
        thread_id: Some(pr_number.to_string()),
        message_thread_id: pr_number.to_string(),
        user_id: authenticated.user_id,
        text: post_merge_ci_text(&execution.run, &execution.owner, &execution.repo, pr_number),
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": execution.owner,
            "repo": execution.repo,
            "pr_number": pr_number,
        }),
        adapter_context: serde_json::json!({}),
        external_event_id: Some(super::external_id::workflow_run(execution.run.run_id)),
        source_task_id: None,
        audience: Audience::SubjectProducer,
    };

    info!(subject_key, "Delivering post-merge CI notification");
    if let Err(error) = super::routing::route_event(state, connection, Some(rule), notice).await {
        warn!(
            subject_key,
            %error,
            "Failed to deliver post-merge CI notification"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PostMergeRun, candidate, post_merge_ci_text};

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

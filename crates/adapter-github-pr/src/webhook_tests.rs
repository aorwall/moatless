use moatless_types::adapter::AdapterKind;
use moatless_loops::adapters::models::SubscriptionMatch;
use moatless_types::adapter::Audience;
use moatless_types::loops::RoutingMode;
use moatless_core::db::DbPool;

use super::{IgnoredWebhook, NormalizeContext, PrimaryWebhook, normalize, title_is_ignored};
use crate::status::WebhookStatusObservation;

fn fake_db_pool() -> DbPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(10))
        .connect_lazy("postgres://fake:fake@localhost:1/fake")
        .unwrap()
}

fn fake_rule() -> SubscriptionMatch {
    use chrono::Utc;
    use moatless_types::access_control::Scope;

    SubscriptionMatch {
        name: "test-loop".to_string(),
        id: "ir_test".into(),
        adapter_kind: AdapterKind::Github,
        connection_id: "pc_test".into(),
        source_matcher: "soaplabs/repo".into(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: "repo-1".into(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        skills: Vec::new(),
        run_as_user_id: Some("alice".into()),
        agent_type: "claude-code".into(),
        inactive_ttl_days: None,
        allow_direct_send: false,
        adapter_config: None,
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

async fn normalize_payload(
    event_type: &str,
    payload: &serde_json::Value,
    rule: &SubscriptionMatch,
) -> super::NormalizedWebhook {
    let db = fake_db_pool();
    normalize(
        event_type,
        payload,
        &NormalizeContext {
            rule,
            owner: "soaplabs",
            repo: "repo",
            sender_login: payload
                .pointer("/sender/login")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("alice"),
            frontend_url: "https://app.example.com",
            db: &db,
        },
    )
    .await
}

fn message(
    primary: Result<PrimaryWebhook, moatless_loops::adapters::error::AdapterError>,
) -> Box<moatless_types::adapter::InboundMessage> {
    match primary.expect("normalization should succeed") {
        PrimaryWebhook::Message(message) => message,
        PrimaryWebhook::CloseTask(_) => panic!("expected message, got close"),
        PrimaryWebhook::Ignore(reason) => panic!("expected message, ignored as {reason:?}"),
    }
}

#[test]
fn title_filters_are_case_insensitive_and_bad_patterns_have_no_effect() {
    let patterns = vec!["^chore\\(deps".to_string(), "^deps[:(]".to_string()];
    assert!(title_is_ignored("CHORE(deps): bump x", &patterns));
    assert!(title_is_ignored("deps(frontend): bump y", &patterns));
    assert!(!title_is_ignored("feat: real feature", &patterns));
    assert!(!title_is_ignored("anything", &["(".to_string()]));
}

#[tokio::test]
async fn opened_pr_preserves_text_audience_external_id_and_subject_facts() {
    let payload = serde_json::json!({
        "action": "opened",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "title": "feat: normalize",
            "html_url": "https://github.com/soaplabs/repo/pull/42",
            "head": {"ref": "feature"},
            "base": {"ref": "main"},
        }
    });
    let normalized = normalize_payload("pull_request", &payload, &fake_rule()).await;
    let candidate = normalized
        .merge_conflict_candidate
        .expect("PR event should carry a conflict candidate");
    assert_eq!(candidate.pr_number, 42);
    let message = message(normalized.primary);

    assert_eq!(message.audience, Audience::Everyone);
    assert_eq!(
        message.external_event_id.as_deref(),
        Some("github:pr:opened:42")
    );
    assert_eq!(message.source_id, "soaplabs/repo");
    assert_eq!(message.user_id, "alice");
    assert_eq!(message.thread_properties["owner"], "soaplabs");
    assert_eq!(message.thread_properties["repo"], "repo");
    assert_eq!(message.thread_properties["pr_number"], 42);
    assert!(message.text.contains("## New Pull Request: #42"));
    assert!(message.text.contains("**Title:** feat: normalize"));
}

#[tokio::test]
async fn comments_preserve_footer_provenance_and_parent_audience() {
    let issue = serde_json::json!({
        "action": "created",
        "sender": {"login": "SoapLabBot"},
        "issue": {"number": 42, "pull_request": {"url": "x"}},
        "comment": {
            "id": 123,
            "body": "Done\n\n---\nMoatless task: [n](https://app.example.com/tasks/task-A)"
        }
    });
    let issue_message = message(
        normalize_payload("issue_comment", &issue, &fake_rule())
            .await
            .primary,
    );
    assert_eq!(issue_message.source_task_id.as_deref(), Some("task-A"));
    assert_eq!(
        issue_message.external_event_id.as_deref(),
        Some("github:ic:123")
    );
    assert_eq!(issue_message.audience, Audience::Everyone);

    let human_issue = serde_json::json!({
        "action": "created",
        "sender": {"login": "alice"},
        "issue": {"number": 42, "pull_request": {"url": "x"}},
        "comment": {"id": 124, "body": "Please re-check this."}
    });
    let human_issue_message = message(
        normalize_payload("issue_comment", &human_issue, &fake_rule())
            .await
            .primary,
    );
    assert_eq!(human_issue_message.source_task_id, None);

    let review = serde_json::json!({
        "action": "created",
        "sender": {"login": "SoapLabBot"},
        "pull_request": {"number": 42},
        "comment": {
            "id": 999,
            "in_reply_to_id": 555,
            "body": "Fixed\n\n---\nMoatless task: [n](https://app.example.com/tasks/task-B)",
            "path": "src/lib.rs"
        }
    });
    let review_message = message(
        normalize_payload("pull_request_review_comment", &review, &fake_rule())
            .await
            .primary,
    );
    assert_eq!(review_message.source_task_id.as_deref(), Some("task-B"));
    assert_eq!(
        review_message.external_event_id.as_deref(),
        Some("github:rc:999")
    );
    assert_eq!(
        review_message.audience,
        Audience::ParentAuthor {
            parent_external_event_id: Some("github:rc:555".into())
        }
    );

    let inline = serde_json::json!({
        "action": "created",
        "sender": {"login": "reviewer"},
        "pull_request": {"number": 42},
        "comment": {"id": 1000, "body": "Rename this.", "path": "src/lib.rs"}
    });
    let inline_message = message(
        normalize_payload("pull_request_review_comment", &inline, &fake_rule())
            .await
            .primary,
    );
    assert_eq!(
        inline_message.audience,
        Audience::ParentAuthor {
            parent_external_event_id: None
        }
    );
}

#[tokio::test]
async fn close_metadata_and_pr_status_are_normalized_together() {
    let mut rule = fake_rule();
    rule.close_task_on_close = true;
    let payload = serde_json::json!({
        "action": "closed",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "state": "closed",
            "merged": true,
            "draft": false,
            "title": "feat: done",
            "html_url": "https://github.com/soaplabs/repo/pull/42"
        }
    });
    let normalized = normalize_payload("pull_request", &payload, &rule).await;
    assert!(normalized.merge_conflict_candidate.is_none());
    assert_eq!(normalized.status_observations.len(), 1);
    match &normalized.status_observations[0] {
        WebhookStatusObservation::PullRequest {
            subject_key,
            status,
        } => {
            assert_eq!(subject_key, "soaplabs/repo#42");
            assert_eq!(status.state_kind.as_deref(), Some("merged"));
            assert_eq!(status.raw["title"], "feat: done");
        }
        WebhookStatusObservation::ReviewDecision { .. } => {
            panic!("expected pull-request status")
        }
    }
    match normalized.primary.expect("close should normalize") {
        PrimaryWebhook::CloseTask(close) => {
            assert_eq!(close.pr_number, 42);
            assert!(close.merged);
            assert_eq!(close.title, "feat: done");
            assert_eq!(close.sender_login, "alice");
        }
        _ => panic!("expected close metadata"),
    }
}

#[tokio::test]
async fn a_merged_pull_request_is_a_candidate_whatever_the_close_does() {
    let payload = serde_json::json!({
        "action": "closed",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "state": "closed",
            "merged": true,
            "draft": false,
            "title": "feat: done",
            "html_url": "https://github.com/soaplabs/repo/pull/42",
            "merged_by": {"login": "maintainer"},
            "base": {"ref": "main"},
            "head": {"ref": "feature"},
            "merge_commit_sha": "abcdef0123456789",
        }
    });

    // A trigger that leaves its tasks open: the primary has no effect at all,
    // and the merge notice is the only thing the pull request produces.
    let keeping = normalize_payload("pull_request", &payload, &fake_rule()).await;
    assert!(matches!(
        keeping.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::NotificationDisabled))
    ));
    let merged = keeping
        .merged_candidate
        .expect("a merged close is a merge notice");
    assert_eq!(merged.pr_number, 42);
    assert_eq!(merged.merged_by, "maintainer");

    // A trigger that closes its tasks: the notice is derived alongside the
    // close, because a task the close does not own still outlives the PR.
    let mut closing = fake_rule();
    closing.close_task_on_close = true;
    let both = normalize_payload("pull_request", &payload, &closing).await;
    assert!(matches!(both.primary, Ok(PrimaryWebhook::CloseTask(_))));
    assert!(both.merged_candidate.is_some());
}

#[tokio::test]
async fn a_pull_request_closed_without_merging_produces_no_merge_notice() {
    let payload = serde_json::json!({
        "action": "closed",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "state": "closed",
            "merged": false,
            "draft": false,
            "title": "feat: abandoned",
            "html_url": "https://github.com/soaplabs/repo/pull/42",
        }
    });
    let normalized = normalize_payload("pull_request", &payload, &fake_rule()).await;
    assert!(normalized.merged_candidate.is_none());
}

#[tokio::test]
async fn ignoring_the_merge_notice_leaves_the_close_alone() {
    let payload = serde_json::json!({
        "action": "closed",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "state": "closed",
            "merged": true,
            "draft": false,
            "title": "feat: done",
            "html_url": "https://github.com/soaplabs/repo/pull/42",
        }
    });
    let mut rule = fake_rule();
    rule.close_task_on_close = true;
    rule.adapter_config = Some(serde_json::json!({"ignored_events": ["pr_merged"]}));
    let normalized = normalize_payload("pull_request", &payload, &rule).await;

    assert!(normalized.merged_candidate.is_none());
    assert!(matches!(
        normalized.primary,
        Ok(PrimaryWebhook::CloseTask(_))
    ));
}

#[tokio::test]
async fn allowlisting_only_the_merge_notice_still_derives_it() {
    // The `pull_request` event is the carrier, so filtering out its own message
    // must not cancel the merge notice a loop asked to receive.
    let mut rule = fake_rule();
    rule.event_filter = serde_json::json!(["pr_merged"]);
    let normalized = normalize_payload(
        "pull_request",
        &serde_json::json!({
            "action": "closed",
            "sender": {"login": "alice"},
            "pull_request": {
                "number": 42,
                "state": "closed",
                "merged": true,
                "draft": false,
                "title": "feat: done",
            }
        }),
        &rule,
    )
    .await;

    assert!(normalized.merged_candidate.is_some());
    assert!(matches!(
        normalized.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered))
    ));
}

#[tokio::test]
async fn review_verdict_preserves_observation_message_and_audience() {
    let payload = serde_json::json!({
        "action": "submitted",
        "sender": {"login": "reviewer"},
        "pull_request": {"number": 42},
        "review": {"id": 77, "state": "changes_requested", "body": ""}
    });
    let normalized = normalize_payload("pull_request_review", &payload, &fake_rule()).await;
    assert_eq!(normalized.status_observations.len(), 1);
    match &normalized.status_observations[0] {
        WebhookStatusObservation::ReviewDecision {
            subject_key,
            decision,
        } => {
            assert_eq!(subject_key, "soaplabs/repo#42");
            assert_eq!(decision.as_str(), "changes_requested");
        }
        WebhookStatusObservation::PullRequest { .. } => {
            panic!("expected review decision")
        }
    }
    let message = message(normalized.primary);
    assert_eq!(message.audience, Audience::Everyone);
    assert_eq!(message.external_event_id.as_deref(), Some("github:rv:77"));
    assert!(message.text.contains("Changes Requested"));
}

#[tokio::test]
async fn failed_check_run_preserves_producer_audience_and_external_id() {
    let payload = serde_json::json!({
        "action": "completed",
        "sender": {"login": "ci-bot"},
        "check_run": {
            "id": 31,
            "name": "CI",
            "conclusion": "failure",
            "html_url": "https://github.com/soaplabs/repo/runs/31",
            "head_sha": "abcdef012345",
            "pull_requests": [{"number": 42}],
            "output": {"summary": "1 test failed"},
        }
    });
    let message = message(
        normalize_payload("check_run", &payload, &fake_rule())
            .await
            .primary,
    );
    assert_eq!(message.audience, Audience::SubjectProducer);
    assert_eq!(message.external_event_id.as_deref(), Some("github:cr:31"));
    assert!(message.text.contains("## CI Check Failed: CI"));
}

#[tokio::test]
async fn partial_pr_payload_retains_observation_before_documented_no_effect_primary() {
    let payload = serde_json::json!({
        "action": "ready_for_review",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 42,
            "state": "open",
            "merged": false,
            "draft": false,
        }
    });
    let normalized = normalize_payload("pull_request", &payload, &fake_rule()).await;

    assert_eq!(normalized.status_observations.len(), 1);
    assert_eq!(
        normalized
            .merge_conflict_candidate
            .expect("all available work remains planned")
            .pr_number,
        42
    );
    assert!(matches!(
        normalized.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::UnsupportedAction))
    ));
    // Ticket 01 documents the current limitation: this partial shape produces
    // a no-effect primary rather than a later parse rejection. Keeping primary
    // as a separate result still pins observations ahead of any future error.
}

#[tokio::test]
async fn config_filters_return_reasons_and_suppress_all_candidates() {
    let payload = serde_json::json!({
        "action": "opened",
        "sender": {"login": "linear-code[bot]"},
        "pull_request": {"number": 42}
    });
    let ignored_sender = normalize_payload("pull_request", &payload, &fake_rule()).await;
    assert!(matches!(
        ignored_sender.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::IgnoredSender))
    ));
    assert!(ignored_sender.status_observations.is_empty());
    assert!(ignored_sender.merge_conflict_candidate.is_none());

    // A sender the loop accepts, so this half isolates event filtering.
    let accepted_sender = serde_json::json!({
        "action": "opened",
        "sender": {"login": "alice"},
        "pull_request": {"number": 42}
    });
    let mut rule = fake_rule();
    rule.event_filter = serde_json::json!(["issue_comment"]);
    let filtered = normalize_payload("pull_request", &accepted_sender, &rule).await;
    assert!(matches!(
        filtered.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered))
    ));
    // Neither derived signal is allowlisted either, so nothing is scheduled.
    assert!(filtered.post_merge_candidate.is_none());
    assert!(filtered.merge_conflict_candidate.is_none());
}

#[tokio::test]
async fn ignoring_a_derived_signal_keeps_the_events_it_is_derived_from() {
    let payload = serde_json::json!({
        "action": "opened",
        "sender": {"login": "alice"},
        "pull_request": {"number": 42}
    });

    let mut rule = fake_rule();
    rule.adapter_config = Some(serde_json::json!({"ignored_events": ["merge_conflict"]}));
    let normalized = normalize_payload("pull_request", &payload, &rule).await;

    // The conflict check is not scheduled — no GitHub call, no FYI message...
    assert!(normalized.merge_conflict_candidate.is_none());
    // ...while the PR event itself still produces its message.
    assert!(matches!(normalized.primary, Ok(PrimaryWebhook::Message(_))));
}

#[tokio::test]
async fn allowlisting_only_a_derived_signal_still_schedules_it() {
    // The raw event is the carrier for the derived signal, so excluding the
    // carrier's own message must not cancel a signal the loop asked for.
    let mut rule = fake_rule();
    rule.event_filter = serde_json::json!(["merge_conflict"]);
    let normalized = normalize_payload(
        "pull_request",
        &serde_json::json!({
            "action": "opened",
            "sender": {"login": "alice"},
            "pull_request": {"number": 42}
        }),
        &rule,
    )
    .await;

    assert_eq!(
        normalized
            .merge_conflict_candidate
            .expect("merge_conflict was allowlisted")
            .pr_number,
        42
    );
    // The pull_request message itself is not allowlisted, so it stays filtered.
    assert!(matches!(
        normalized.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::EventFiltered))
    ));
}

#[tokio::test]
async fn allowlisting_only_post_merge_still_schedules_it() {
    let mut rule = fake_rule();
    rule.event_filter = serde_json::json!(["post_merge"]);
    let normalized = normalize_payload(
        "workflow_run",
        &serde_json::json!({
            "action": "completed",
            "sender": {"login": "alice"},
            "workflow_run": {
                "id": 7,
                "name": "CI",
                "event": "push",
                "conclusion": "failure",
                "head_sha": "abc123",
                "head_branch": "main",
                "html_url": "https://github.com/soaplabs/repo/actions/runs/7",
                "pull_requests": []
            }
        }),
        &rule,
    )
    .await;

    assert!(
        normalized.post_merge_candidate.is_some(),
        "post_merge was allowlisted"
    );
}

#[tokio::test]
async fn configured_sender_and_title_filters_preserve_opt_out_behavior() {
    let bot_comment = serde_json::json!({
        "action": "created",
        "sender": {"login": "linear-code[bot]"},
        "issue": {"number": 42, "pull_request": {"url": "x"}},
        "comment": {"id": 123, "body": "Linked"}
    });
    let mut custom_ignore = fake_rule();
    custom_ignore.adapter_config =
        Some(serde_json::json!({"ignored_senders": ["Linear-Code[Bot]"]}));
    let ignored = normalize_payload("issue_comment", &bot_comment, &custom_ignore).await;
    assert!(matches!(
        ignored.primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::IgnoredSender))
    ));

    let mut opt_out = fake_rule();
    opt_out.adapter_config = Some(serde_json::json!({"ignored_senders": []}));
    assert!(matches!(
        normalize_payload("issue_comment", &bot_comment, &opt_out)
            .await
            .primary,
        Ok(PrimaryWebhook::Message(_))
    ));

    let dependency_pr = serde_json::json!({
        "action": "opened",
        "sender": {"login": "dependabot[bot]"},
        "pull_request": {
            "number": 42,
            "title": "chore(deps): bump serde",
            "head": {"ref": "deps"},
            "base": {"ref": "main"},
        }
    });
    let mut title_filter = fake_rule();
    title_filter.adapter_config =
        Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps"]}));
    assert!(matches!(
        normalize_payload("pull_request", &dependency_pr, &title_filter)
            .await
            .primary,
        Ok(PrimaryWebhook::Ignore(IgnoredWebhook::IgnoredTitle))
    ));

    let feature_pr = serde_json::json!({
        "action": "opened",
        "sender": {"login": "alice"},
        "pull_request": {
            "number": 43,
            "title": "feat: keep this",
            "head": {"ref": "feature"},
            "base": {"ref": "main"},
        }
    });
    assert!(matches!(
        normalize_payload("pull_request", &feature_pr, &title_filter)
            .await
            .primary,
        Ok(PrimaryWebhook::Message(_))
    ));
}

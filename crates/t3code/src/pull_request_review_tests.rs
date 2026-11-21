use super::*;

fn overview() -> PullRequestOverview {
    PullRequestOverview {
        number: 7,
        title: "Move the picker".into(),
        body: "Why it moves.".into(),
        url: "https://github.com/acme/app/pull/7".into(),
        state: "open".into(),
        head_ref: "feat/picker".into(),
        head_sha: "headsha".into(),
        base_ref: "main".into(),
        base_sha: "basesha".into(),
        created_at: Some("2026-10-01T09:00:00Z".into()),
        ..Default::default()
    }
}

fn target() -> PullRequestTarget<'static> {
    PullRequestTarget {
        project_id: "ws-1",
        repository: "Acme/App",
        number: 7,
    }
}

#[test]
fn detail_offers_nothing_the_viewer_may_not_do() {
    let detail = detail_json(
        &target(),
        "Acme",
        &overview(),
        &PullRequestStatus::default(),
    );
    assert_eq!(detail["capabilities"]["diff"], true);
    assert_eq!(detail["viewerPermissions"]["actions"], json!([]));
    assert_eq!(detail["viewerPermissions"]["comment"], false);
    assert_eq!(detail["workspaceRoot"], nominal_path::nominal_root("ws-1"));
    assert_eq!(detail["repository"], "Acme/App");
    assert_eq!(detail["headSha"], "headsha");
    assert_eq!(detail["updatedAt"], detail["createdAt"]);
    assert_eq!(detail["author"], Value::Null);
    assert_eq!(detail["mergeability"], "unknown");
}

#[test]
fn detail_carries_the_status_read_beside_it() {
    use moatless_types::pull_request_conversation::{
        PullRequestActor, PullRequestCheck, PullRequestCheckStatus, PullRequestLabel,
    };
    let status = PullRequestStatus {
        labels: vec![PullRequestLabel {
            name: "bug".into(),
            color: Some("d73a4a".into()),
        }],
        requested_reviewers: vec![PullRequestActor {
            login: "copilot".into(),
            is_bot: true,
            ..Default::default()
        }],
        checks: vec![PullRequestCheck {
            name: "lint".into(),
            status: PullRequestCheckStatus::ActionRequired,
            description: None,
            url: None,
            required: None,
        }],
        viewer: None,
    };
    let detail = detail_json(&target(), "Acme", &overview(), &status);
    assert_eq!(
        detail["labels"],
        json!([{ "name": "bug", "color": "d73a4a" }])
    );
    assert_eq!(
        detail["reviewers"],
        json!([{ "login": "copilot", "name": null, "avatarUrl": null, "isBot": true }])
    );
    assert_eq!(
        detail["checks"],
        json!([{ "name": "lint", "status": "action-required", "description": null, "url": null }])
    );
}

/// `ws-1` places `project_repository`, or `acme/api` then `acme/app` when it
/// is `"acme/api+acme/app"`; no Task is bound to
/// the pull request, so a fallback finds nothing. `fallback` is how many
/// times the bindings are asked.
async fn reach_with(
    project_repository: Option<&str>,
    fallback: usize,
) -> Result<Option<String>, PullRequestReviewError> {
    reach_with_tasks(project_repository, &[], fallback).await
}

/// [`reach_with`], where `ws-1`'s readable Tasks work in `task_repositories`.
async fn reach_with_tasks(
    project_repository: Option<&str>,
    task_repositories: &[&str],
    fallback: usize,
) -> Result<Option<String>, PullRequestReviewError> {
    let mut host =
        crate::pull_request_list::tests::ProjectRepositories::new(match project_repository {
            Some("acme/app") => |_| Ok(vec!["acme/app".to_string()]),
            Some("acme/api+acme/app") => |_| Ok(vec!["acme/api".into(), "acme/app".into()]),
            Some(_) => |_| Ok(vec!["acme/api".to_string()]),
            None => |_| Ok(Vec::new()),
        });
    host.task_repositories = task_repositories.iter().map(ToString::to_string).collect();
    let mut bound = crate::ports::MockBoundPullRequestSource::new();
    bound
        .expect_tasks_bound_to()
        .times(fallback)
        .returning(|_, _| Vec::new());
    let conversations = crate::dao::MockConversationDao::new();
    let viewer = ScopeContext::user("user-1".to_string());
    reach_through(&host, &conversations, &bound, &viewer, &target()).await
}

#[tokio::test]
async fn a_pull_request_in_the_projects_primary_repository_is_reached_through_it() {
    let reached = reach_with(Some("acme/app"), 0).await.unwrap();
    assert_eq!(reached.as_deref(), Some("ws-1"));
}

#[tokio::test]
async fn a_pull_request_in_a_projects_other_repository_is_reached_through_it() {
    let reached = reach_with(Some("acme/api+acme/app"), 0).await.unwrap();
    assert_eq!(reached.as_deref(), Some("ws-1"));
}

#[tokio::test]
async fn a_pull_request_in_another_repository_needs_a_bound_task() {
    let reached = reach_with(Some("acme/api"), 1).await;
    assert!(matches!(reached, Err(PullRequestReviewError::Unknown)));
}

#[tokio::test]
async fn an_unreadable_project_falls_back_to_a_bound_task() {
    let reached = reach_with(None, 1).await;
    assert!(matches!(reached, Err(PullRequestReviewError::Unknown)));
}

#[tokio::test]
async fn a_pull_request_in_a_repository_a_task_works_in_is_reached_through_the_project() {
    let reached = reach_with_tasks(Some("acme/api"), &["Acme/App"], 0)
        .await
        .unwrap();
    assert_eq!(reached.as_deref(), Some("ws-1"));
}

#[test]
fn a_writer_is_offered_what_github_lets_them_do() {
    use moatless_types::pull_request_conversation::PullRequestViewerRights;
    let status = PullRequestStatus {
        viewer: Some(PullRequestViewerRights {
            can_write: true,
            can_triage: true,
            can_update: true,
            can_merge_now: true,
            auto_merge_allowed: true,
            squash_merge_allowed: true,
            auto_merge_method: Some("squash".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let detail = detail_json(&target(), "Acme", &overview(), &status);
    let permissions = &detail["viewerPermissions"];
    // Mergeable now, so no auto-merge to arm; the branch is not theirs.
    assert_eq!(
        permissions["actions"],
        json!([
            "merge",
            "disable-auto-merge",
            "ready",
            "draft",
            "close",
            "reopen"
        ])
    );
    assert_eq!(permissions["resolve"], true);
    assert_eq!(permissions["labels"], true);
    assert!(permissions.get("updateMethods").is_none());
    assert_eq!(
        detail["mergeCapabilities"],
        json!({ "merge": false, "squash": true, "rebase": false })
    );
    assert_eq!(detail["autoMergeEnabled"], true);
    assert_eq!(detail["autoMergeMethod"], "squash");
}

#[test]
fn an_author_cannot_approve_their_own_pull_request() {
    use moatless_types::pull_request_conversation::PullRequestViewerRights;
    let status = PullRequestStatus {
        viewer: Some(PullRequestViewerRights {
            did_author: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let detail = detail_json(&target(), "Acme", &overview(), &status);
    let permissions = &detail["viewerPermissions"];
    assert_eq!(permissions["verdicts"], json!(["comment"]));
    assert_eq!(permissions["resolve"], true);
    assert_eq!(permissions["requestReviewers"], false);
    assert_eq!(permissions["actions"], json!([]));
}

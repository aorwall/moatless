use super::*;

use crate::dao::MockConversationDao;
use crate::ports::MockIssueTrackerSource;
use crate::workspace_listing::MockWorkspaceListing;
use moatless_types::issue_tracker::IssueActor;
use moatless_workspaces::workspace::models::ListedWorkspace;

fn viewer() -> ScopeContext {
    ScopeContext::new("user-1".to_string(), false)
}

fn project(id: &str, name: &str, team: Option<&str>) -> ListedWorkspace {
    let mut workspace = moatless_workspaces::test_support::make_workspace_row(id, "user-1");
    workspace.name = name.to_string();
    workspace.linear_team = team.map(str::to_string);
    ListedWorkspace {
        readable: true,
        workspace,
    }
}

fn workspaces(listed: Vec<ListedWorkspace>) -> MockWorkspaceListing {
    let mut workspaces = MockWorkspaceListing::new();
    workspaces
        .expect_list_readable_including()
        .returning(move |_, _| Ok(listed.clone()));
    workspaces
}

fn issue(key: &str, updated_at: &str) -> IssueSummary {
    IssueSummary {
        scope: key
            .split_once('-')
            .map_or(key, |(team, _)| team)
            .to_string(),
        key: key.to_string(),
        title: format!("Issue {key}"),
        url: format!("https://linear.app/acme/issue/{key}"),
        author: Some(IssueActor {
            login: "alice".to_string(),
            name: Some("Alice".to_string()),
            avatar_url: None,
            is_bot: false,
        }),
        state: IssueState {
            name: "Done".to_string(),
            closed: true,
            close_reason: Some("completed".to_string()),
        },
        created_at: "2026-11-01T09:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
        closed_at: None,
        assignees: Vec::new(),
        labels: vec![IssueLabel {
            name: "bug".to_string(),
            color: Some("#ff0000".to_string()),
            description: None,
        }],
        comment_count: 0,
    }
}

fn request(payload: Value) -> IssueRequest {
    serde_json::from_value(payload).expect("an IssueRequest")
}

#[tokio::test]
async fn a_listing_reads_each_declared_team_once_through_its_first_project() {
    let listed = workspaces(vec![
        project("ws-1", "App", Some("sp")),
        project("ws-2", "Docs", Some("SP")),
        project("ws-3", "Infra", Some("OPS")),
        project("ws-4", "Unbound", None),
    ]);
    let mut tracker = MockIssueTrackerSource::new();
    tracker
        .expect_list()
        .times(2)
        .returning(|_, query, viewer_id| {
            assert_eq!(viewer_id, None);
            match query.scope.as_str() {
                "SP" => Ok(IssuePage {
                    items: vec![issue("SP-1", "2026-11-02T09:00:00Z")],
                    next_cursor: Some("next".to_string()),
                }),
                "OPS" => Ok(IssuePage {
                    items: vec![issue("OPS-4", "2026-11-03T09:00:00Z")],
                    next_cursor: None,
                }),
                other => panic!("unexpected team {other}"),
            }
        });
    let conversations = MockConversationDao::new();
    let ports = IssuePorts {
        workspaces: &listed,
        tracker: &tracker,
        conversations: &conversations,
    };

    let result = list(&ports, &viewer(), &IssueListInput::default())
        .await
        .unwrap();

    let keys: Vec<&str> = result["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["OPS-4", "SP-1"]);
    assert_eq!(result["entries"][1]["projectId"], "ws-1");
    assert_eq!(result["entries"][1]["state"]["category"], "closed");
    assert_eq!(result["nextCursors"], json!({ "linear.app SP": "next" }));
    assert_eq!(result["trackers"][0]["projectCount"], 2);
    assert_eq!(
        result["scopes"],
        json!([
            { "projectId": "ws-1", "tracker": "linear", "host": "linear.app", "scope": "SP" },
            { "projectId": "ws-3", "tracker": "linear", "host": "linear.app", "scope": "OPS" },
        ])
    );
}

#[tokio::test]
async fn a_continuation_reads_only_the_teams_it_names() {
    let listed = workspaces(vec![
        project("ws-1", "App", Some("SP")),
        project("ws-3", "Infra", Some("OPS")),
    ]);
    let mut tracker = MockIssueTrackerSource::new();
    tracker.expect_list().times(1).returning(|_, query, _| {
        assert_eq!(query.scope, "OPS");
        assert_eq!(query.after.as_deref(), Some("cursor-2"));
        Ok(IssuePage {
            items: Vec::new(),
            next_cursor: None,
        })
    });
    let conversations = MockConversationDao::new();
    let ports = IssuePorts {
        workspaces: &listed,
        tracker: &tracker,
        conversations: &conversations,
    };
    let input: IssueListInput = serde_json::from_value(json!({
        "state": "open",
        "cursors": { "linear.app OPS": "cursor-2" },
    }))
    .unwrap();

    list(&ports, &viewer(), &input).await.unwrap();
}

#[tokio::test]
async fn an_issue_outside_the_named_projects_team_is_refused_before_linear_is_asked() {
    let listed = workspaces(vec![project("ws-1", "App", Some("SP"))]);
    let tracker = MockIssueTrackerSource::new();
    let conversations = MockConversationDao::new();
    let ports = IssuePorts {
        workspaces: &listed,
        tracker: &tracker,
        conversations: &conversations,
    };

    let failure = detail(
        &ports,
        &viewer(),
        &request(json!({
            "projectId": "ws-1",
            "tracker": "linear",
            "scope": "OPS",
            "key": "OPS-1",
        })),
    )
    .await
    .unwrap_err();

    assert!(matches!(failure, IssueFailure::NotBound));
    assert_eq!(
        failure_json("issues.detail", &failure),
        json!({
            "_tag": "IssueUnavailableError",
            "reason": "tracker-unsupported",
            "tracker": "linear",
        })
    );
}

#[tokio::test]
async fn a_comment_goes_out_under_the_viewers_login_on_the_projects_team() {
    let listed = workspaces(vec![project("ws-1", "App", Some("SP"))]);
    let mut tracker = MockIssueTrackerSource::new();
    tracker
        .expect_comment()
        .withf(|_, login, scope, key, body| {
            login == "alice" && scope == "SP" && key == "SP-12" && body == "Looks good"
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    let conversations = MockConversationDao::new();
    let ports = IssuePorts {
        workspaces: &listed,
        tracker: &tracker,
        conversations: &conversations,
    };

    let answer = comment(
        &ports,
        &viewer(),
        "alice",
        &request(json!({
            "projectId": "ws-1",
            "tracker": "linear",
            "scope": "sp",
            "key": "SP-12",
            "body": "Looks good",
        })),
    )
    .await
    .unwrap();

    assert_eq!(answer, Value::Null);
}

#[test]
fn a_missing_issue_answers_as_not_found() {
    let failure = IssueFailure::Tracker(IssueTrackerError::NotFound);
    assert_eq!(
        failure_json("issues.detail", &failure)["reason"],
        "not-found"
    );
}

#[tokio::test]
async fn the_tracker_listing_binds_each_project_to_its_team() {
    let listed = workspaces(vec![
        project("ws-1", "App", Some("SP")),
        project("ws-4", "Unbound", None),
    ]);
    let mut tracker = MockIssueTrackerSource::new();
    tracker.expect_workspaces().returning(|_| {
        Ok(vec![moatless_types::issue_tracker::IssueTrackerWorkspace {
            url_key: "acme".to_string(),
            name: "Acme".to_string(),
            teams: vec![moatless_types::issue_tracker::IssueTrackerTeam {
                key: "SP".to_string(),
                name: "Product".to_string(),
            }],
        }])
    });
    let conversations = MockConversationDao::new();
    let ports = IssuePorts {
        workspaces: &listed,
        tracker: &tracker,
        conversations: &conversations,
    };

    let result = trackers(&ports, &viewer()).await.unwrap();

    assert_eq!(
        result["bindings"],
        json!({ "ws-1": { "tracker": "linear", "scope": "SP" } })
    );
    assert_eq!(result["trackers"][0]["workspace"], "acme");
    assert_eq!(result["trackers"][0]["connected"], true);
    assert_eq!(
        result["trackers"][0]["scopes"],
        json!([{ "key": "SP", "name": "Product" }])
    );
}

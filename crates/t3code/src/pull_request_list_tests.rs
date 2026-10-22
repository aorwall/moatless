use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::workspace_listing::MockWorkspaceListing;
use async_trait::async_trait;

fn input(payload: Value) -> PullRequestListInput {
    serde_json::from_value(payload).expect("a PullRequestListInput")
}

fn source() -> Source {
    Source {
        project_id: "ws-1".into(),
        project_title: "App".into(),
        repository: "acme/app".into(),
        cursor: None,
    }
}

fn item() -> PullRequestListItem {
    PullRequestListItem {
        number: 7,
        title: "Move the picker".into(),
        url: "https://github.com/acme/app/pull/7".into(),
        author_login: Some("alice".into()),
        head_ref: "feat/picker".into(),
        base_ref: "main".into(),
        state: "open".into(),
        mergeability: "unknown".into(),
        created_at: "2026-10-01T09:00:00Z".into(),
        updated_at: "2026-10-02T09:00:00Z".into(),
        labels: vec![("ui".into(), Some("ff0000".into()))],
        review_decision: Some("approved".into()),
        requested_reviewers: vec!["Bob".into()],
        ..Default::default()
    }
}

#[test]
fn the_input_becomes_a_query() {
    let query = list_query(&input(json!({
        "state": "merged",
        "involvement": "authored",
        "query": " picker ",
        "filters": {
            "draft": "hide",
            "review": "none",
            "checks": "failing",
            "labels": [["bug", "ui"], ["p1"]],
            "excludedLabels": ["wontfix"],
            "author": "alice",
        },
    })));
    assert_eq!(query.state, "merged");
    assert_eq!(query.involvement.as_deref(), Some("authored"));
    assert_eq!(query.text.as_deref(), Some("picker"));
    assert_eq!(query.draft.as_deref(), Some("hide"));
    assert_eq!(query.review.as_deref(), Some("none"));
    assert_eq!(query.checks.as_deref(), Some("failing"));
    assert_eq!(
        query.labels,
        vec![vec!["bug".to_string(), "ui".into()], vec!["p1".into()]]
    );
    assert_eq!(query.excluded_labels, vec!["wontfix".to_string()]);
    assert_eq!(query.author.as_deref(), Some("alice"));
    assert_eq!(list_query(&input(json!({}))).state, "open");
}

#[test]
fn an_item_becomes_a_list_entry() {
    let entry = entry(&source(), &item(), Some("bob")).unwrap();
    assert_eq!(entry["projectId"], "ws-1");
    assert_eq!(entry["host"], "github.com");
    assert_eq!(entry["author"]["login"], "alice");
    assert_eq!(entry["viewerReviewRequested"], true);
    assert_eq!(
        entry["labels"],
        json!([{ "name": "ui", "color": "ff0000" }])
    );
    assert_eq!(entry["reviewDecision"], "approved");
    assert!(entry.get("checksState").is_none());
}

#[test]
fn an_author_is_not_asked_to_review_their_own_change() {
    let mut own = item();
    own.author_login = Some("bob".into());
    own.requested_reviewers = vec!["bob".into()];
    assert_eq!(
        entry(&source(), &own, Some("bob")).unwrap()["viewerReviewRequested"],
        false
    );
}

#[test]
fn a_row_missing_a_required_field_is_dropped() {
    let mut broken = item();
    broken.url = String::new();
    assert!(entry(&source(), &broken, None).is_none());
}

#[test]
fn a_continuation_is_reported_as_truncated() {
    let mut cursors = Map::new();
    cursors.insert("github.com acme/app".into(), json!("Y3Vy"));
    let answer = result(&BTreeMap::new(), 1, Vec::new(), Vec::new(), cursors);
    assert_eq!(answer["truncated"], true);
    assert_eq!(answer["providers"][0]["projectCount"], 1);
    assert_eq!(answer["nextCursors"]["github.com acme/app"], "Y3Vy");
    let empty = result(&BTreeMap::new(), 0, Vec::new(), Vec::new(), Map::new());
    assert_eq!(empty["providers"], json!([]));
    assert_eq!(empty["truncated"], false);
}

fn project(id: &str, name: &str) -> moatless_workspaces::workspace::models::ListedWorkspace {
    let mut workspace = moatless_workspaces::test_support::make_workspace_row(id, "user-1");
    workspace.name = name.to_string();
    moatless_workspaces::workspace::models::ListedWorkspace {
        readable: true,
        workspace,
    }
}

fn projects(
    listed: Vec<moatless_workspaces::workspace::models::ListedWorkspace>,
) -> MockWorkspaceListing {
    let mut workspaces = MockWorkspaceListing::new();
    workspaces
        .expect_list_readable_including()
        .returning(move |_, _| Ok(listed.clone()));
    workspaces
}

/// A host whose projects' primary repositories `repository_of` names, and
/// which counts the listings asked of it and refuses them.
pub(crate) struct ProjectRepositories {
    pub(crate) repository_of: fn(&str) -> Result<Option<String>, PullRequestError>,
    /// What every project's readable Tasks work in.
    pub(crate) task_repositories: Vec<String>,
    pub(crate) listed: AtomicUsize,
}

impl ProjectRepositories {
    pub(crate) fn new(repository_of: fn(&str) -> Result<Option<String>, PullRequestError>) -> Self {
        Self {
            repository_of,
            task_repositories: Vec::new(),
            listed: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl PullRequestReviewSource for ProjectRepositories {
    async fn project_repository(
        &self,
        _viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, PullRequestError> {
        (self.repository_of)(workspace_id)
    }
    async fn task_repositories(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
    ) -> Result<Vec<String>, PullRequestError> {
        Ok(self.task_repositories.clone())
    }
    async fn list(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
        _repository: &str,
        _query: &PullRequestListQuery,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<PullRequestListPage, PullRequestError> {
        self.listed.fetch_add(1, Ordering::SeqCst);
        Err(PullRequestError::Failed("not listed in this test".into()))
    }
    async fn overview(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<crate::ports::PullRequestOverview, PullRequestError> {
        unreachable!("no test here reads a pull request")
    }
    async fn patch_page(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _page: u32,
    ) -> Result<crate::ports::PullRequestPatchPage, PullRequestError> {
        unreachable!("no test here reads a pull request")
    }
    async fn commit_patch_page(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _sha: &str,
        _page: u32,
    ) -> Result<crate::ports::PullRequestPatchPage, crate::ports::PullRequestError> {
        unreachable!("no test here reads a pull request")
    }
    async fn activity(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<
        moatless_types::pull_request_conversation::PullRequestActivity,
        crate::ports::PullRequestError,
    > {
        unreachable!("no test here reads a pull request")
    }
    async fn thread_comments(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _thread_id: &str,
        _cursor: Option<&str>,
    ) -> Result<
        moatless_types::pull_request_conversation::ReviewThreadCommentPage,
        crate::ports::PullRequestError,
    > {
        unreachable!("no test here reads a pull request")
    }
    async fn status(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<
        moatless_types::pull_request_conversation::PullRequestStatus,
        crate::ports::PullRequestError,
    > {
        unreachable!("no test here reads a pull request")
    }
    async fn file_contents(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _file: &crate::ports::ChangedFile<'_>,
    ) -> Result<
        moatless_types::pull_request_conversation::FileContentsPair,
        crate::ports::PullRequestError,
    > {
        unreachable!("no test here reads a pull request")
    }
    async fn create_review_guide_task(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
        _name: &str,
        _message: String,
    ) -> Result<String, PullRequestError> {
        unreachable!("no test here writes a guide")
    }
    async fn fork_review_guide_task(
        &self,
        _viewer: &ScopeContext,
        _source_task_id: &str,
        _task_id: &str,
        _name: &str,
        _message: String,
    ) -> Result<bool, PullRequestError> {
        unreachable!("no test here writes a guide")
    }
}

/// `ws-1` and `ws-2` share `acme/app`, `ws-3` lists `acme/api`, `ws-4` is not
/// on GitHub and `ws-5`'s repository could not be read.
fn repositories() -> ProjectRepositories {
    ProjectRepositories::new(|project_id| match project_id {
        "ws-1" => Ok(Some("acme/app".into())),
        "ws-2" => Ok(Some("Acme/App".into())),
        "ws-3" => Ok(Some("acme/api".into())),
        "ws-5" => Err(PullRequestError::Unavailable("The store is down.".into())),
        _ => Ok(None),
    })
}

fn every_project() -> MockWorkspaceListing {
    let mut unreadable = project("ws-6", "Hidden");
    unreadable.readable = false;
    projects(vec![
        project("ws-1", "App"),
        project("ws-2", "App again"),
        project("ws-3", "Api"),
        project("ws-4", "Elsewhere"),
        project("ws-5", "Broken"),
        unreadable,
    ])
}

fn repositories_of(sources: &[Source]) -> Vec<(&str, &str)> {
    sources
        .iter()
        .map(|source| (source.project_id.as_str(), source.repository.as_str()))
        .collect()
}

#[tokio::test]
async fn each_readable_repository_is_read_once_and_a_failure_is_reported() {
    let viewer = ScopeContext::user("user-1".to_string());
    let (sources, errors) = sources(&every_project(), &repositories(), &viewer, &[], None)
        .await
        .unwrap();
    assert_eq!(
        repositories_of(&sources),
        vec![("ws-1", "acme/app"), ("ws-3", "acme/api")]
    );
    assert_eq!(
        errors,
        vec![json!({
            "projectId": "ws-5",
            "projectTitle": "Broken",
            "message": "The store is down.",
        })]
    );
}

#[tokio::test]
async fn named_projects_narrow_the_listing() {
    let viewer = ScopeContext::user("user-1".to_string());
    let named = projects_named(&input(
        json!({ "projectIds": ["ws-3"], "projectId": "ws-2" }),
    ));
    let (sources, _) = sources(&every_project(), &repositories(), &viewer, &named, None)
        .await
        .unwrap();
    assert_eq!(
        repositories_of(&sources),
        vec![("ws-2", "Acme/App"), ("ws-3", "acme/api")]
    );
}

#[tokio::test]
async fn a_continuation_reads_only_the_repositories_it_names() {
    let viewer = ScopeContext::user("user-1".to_string());
    let mut cursors = BTreeMap::new();
    cursors.insert("github.com acme/api".to_string(), "Y3Vy".to_string());
    let (sources, _) = sources(
        &every_project(),
        &repositories(),
        &viewer,
        &[],
        Some(&cursors),
    )
    .await
    .unwrap();
    assert_eq!(repositories_of(&sources), vec![("ws-3", "acme/api")]);
    assert_eq!(sources[0].cursor.as_deref(), Some("Y3Vy"));
}

#[tokio::test]
async fn involvement_without_the_viewers_own_login_is_refused() {
    let viewer = ScopeContext::user("user-1".to_string());
    let host = repositories();
    let query = PullRequestListQuery {
        involvement: Some("authored".into()),
        ..Default::default()
    };
    let (_, page) = read(&host, &viewer, &query, 10, &source()).await;
    assert!(matches!(page, Err(PullRequestError::Unavailable(message)) if message == NO_OWN_LOGIN));
    assert_eq!(host.listed.load(Ordering::SeqCst), 0);
}

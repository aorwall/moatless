//! A GitHub REST stand-in on a loopback port, serving the repository files,
//! listings and repository reads a test puts in it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::Value;

#[derive(Default)]
struct Contents {
    /// Keyed by `owner/repo/path`.
    files: HashMap<String, String>,
    /// Keyed by organization; served as one page.
    orgs: HashMap<String, Vec<Value>>,
    /// `GET /repos/{owner}/{repo}`, keyed by `owner/repo`.
    repos: HashMap<String, Value>,
    /// The token's user and every repository `/user/repos` lists for it.
    user: Option<(String, Vec<Value>)>,
    /// `/users/{account}/repos`, keyed by account.
    users: HashMap<String, Vec<Value>>,
}

#[derive(Clone)]
pub(crate) struct FakeGithub {
    /// The `github_api_base` to point a client at.
    pub(crate) base: String,
    contents: Arc<Mutex<Contents>>,
}

impl FakeGithub {
    pub(crate) async fn start() -> Self {
        let contents = Arc::new(Mutex::new(Contents::default()));
        let app = Router::new()
            .route("/repos/{owner}/{repo}/contents/{*path}", get(file))
            .route("/orgs/{org}/repos", get(org_repos))
            .route("/repos/{owner}/{repo}", get(repo))
            .route("/user", get(user))
            .route("/user/repos", get(user_repos))
            .route("/users/{account}/repos", get(account_repos))
            .with_state(contents.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, contents }
    }

    /// Serve `body` as `path` in the repository `owner/repo`.
    pub(crate) fn put_file(&self, owner_repo: &str, path: &str, body: &str) {
        self.contents
            .lock()
            .unwrap()
            .files
            .insert(format!("{owner_repo}/{path}"), body.to_string());
    }

    /// List `repos`, GitHub repository objects, as `org`'s.
    pub(crate) fn set_org(&self, org: &str, repos: Vec<Value>) {
        self.contents
            .lock()
            .unwrap()
            .orgs
            .insert(org.to_string(), repos);
    }

    /// Answer `GET /repos/{owner_repo}` with `repo`.
    pub(crate) fn set_repo(&self, owner_repo: &str, repo: Value) {
        self.contents
            .lock()
            .unwrap()
            .repos
            .insert(owner_repo.to_string(), repo);
    }

    /// Make the token `login`'s, owning `repos`.
    pub(crate) fn set_user(&self, login: &str, repos: Vec<Value>) {
        self.contents.lock().unwrap().user = Some((login.to_string(), repos));
    }

    /// List `repos` as the public repositories of `account`.
    pub(crate) fn set_account(&self, account: &str, repos: Vec<Value>) {
        self.contents
            .lock()
            .unwrap()
            .users
            .insert(account.to_string(), repos);
    }
}

type Shared = Arc<Mutex<Contents>>;

async fn file(
    State(contents): State<Shared>,
    Path((owner, repo, path)): Path<(String, String, String)>,
) -> Response {
    let key = format!("{owner}/{repo}/{path}");
    match contents.lock().unwrap().files.get(&key) {
        Some(body) => body.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `repos` as page 1 of a listing, or 404 for an unknown account.
fn page(repos: Option<Vec<Value>>, query: &HashMap<String, String>) -> Response {
    let Some(repos) = repos else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let page = query.get("page").map_or("1", String::as_str);
    Json(if page == "1" { repos } else { Vec::new() }).into_response()
}

async fn org_repos(
    State(contents): State<Shared>,
    Path(org): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let repos = contents.lock().unwrap().orgs.get(&org).cloned();
    page(repos, &query)
}

async fn repo(
    State(contents): State<Shared>,
    Path((owner, repo)): Path<(String, String)>,
) -> Response {
    match contents
        .lock()
        .unwrap()
        .repos
        .get(&format!("{owner}/{repo}"))
    {
        Some(repo) => Json(repo.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn user(State(contents): State<Shared>) -> Response {
    match &contents.lock().unwrap().user {
        Some((login, _)) => Json(serde_json::json!({ "login": login })).into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn user_repos(
    State(contents): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let repos = contents
        .lock()
        .unwrap()
        .user
        .as_ref()
        .map(|(_, r)| r.clone());
    page(repos, &query)
}

async fn account_repos(
    State(contents): State<Shared>,
    Path(account): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let repos = contents.lock().unwrap().users.get(&account).cloned();
    page(repos, &query)
}

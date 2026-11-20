use std::process::{Command, Stdio};
use std::sync::Mutex;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use super::*;

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";
const RACER: &str = "3333333333333333333333333333333333333333";

fn pull(number: u64, sha: &str) -> Pull {
    Pull {
        number,
        head: PullHead {
            sha: sha.to_string(),
        },
    }
}

#[test]
fn a_branch_at_the_pushed_commit_with_a_pull_request_behind_it_is_unprocessed() {
    let pulls = [pull(1917, OLD), pull(1918, NEW)];
    assert_eq!(
        verdict(Some(NEW), &pulls, NEW),
        Verdict::Unprocessed(vec![1917])
    );
    assert_eq!(verdict(Some(NEW), &pulls[1..], NEW), Verdict::Processed);
    assert_eq!(verdict(Some(NEW), &[], NEW), Verdict::Processed);
    // A later push, or a rejected one: the pull request lagging is not this
    // push's to repair.
    assert_eq!(verdict(Some(RACER), &pulls, NEW), Verdict::Moved);
    assert_eq!(verdict(None, &pulls, NEW), Verdict::Moved);
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn receive_pack(repo: &std::path::Path, request: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut child = Command::new("git")
        .args(["receive-pack", "--stateless-rpc", "."])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("git receive-pack runs");
    child.stdin.take().unwrap().write_all(request).unwrap();
    child.wait_with_output().unwrap().stdout
}

#[test]
fn real_receive_pack_applies_the_update_only_from_the_head_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q", "-b", "feature"]);
    git(repo, &["config", "receive.denyCurrentBranch", "ignore"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "old"]);
    let old = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "new"]);
    let new = git(repo, &["rev-parse", "HEAD"]);
    let ref_name = "refs/heads/feature";

    let back = receive_pack(repo, &update_request(ref_name, &new, &old));
    assert_eq!(read_report(&back, ref_name), Update::Done);
    assert_eq!(git(repo, &["rev-parse", ref_name]), old);

    // The branch is no longer at `new`, so the same update is refused.
    let stale = receive_pack(repo, &update_request(ref_name, &new, &old));
    assert!(matches!(read_report(&stale, ref_name), Update::Refused(_)));
    assert_eq!(git(repo, &["rev-parse", ref_name]), old);

    let forward = receive_pack(repo, &update_request(ref_name, &old, &new));
    assert_eq!(read_report(&forward, ref_name), Update::Done);
    assert_eq!(git(repo, &["rev-parse", ref_name]), new);
}

/// GitHub as the watch sees it: one branch, one pull request on it, and
/// receive-pack updating the branch under a lease.
#[derive(Default)]
struct FakeGitHub {
    branch: String,
    pull_head: String,
    /// Whether a push moves the pull request's head, as a processed one does.
    processes_pushes: bool,
    /// Moved onto the branch right after the first update, as a racing push.
    racer: Option<String>,
    /// Apply the first update but answer 502, as a lost response.
    lose_first_answer: bool,
    updates: Vec<(String, String)>,
}

type Shared = Arc<Mutex<FakeGitHub>>;

async fn branch_ref(State(github): State<Shared>) -> Response {
    let sha = github.lock().unwrap().branch.clone();
    axum::Json(serde_json::json!({ "object": { "sha": sha } })).into_response()
}

async fn pulls(
    State(github): State<Shared>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    assert_eq!(
        query.get("head").map(String::as_str),
        Some("soaplabs:feat/x")
    );
    assert_eq!(query.get("state").map(String::as_str), Some("open"));
    let head = github.lock().unwrap().pull_head.clone();
    axum::Json(serde_json::json!([{ "number": 1917, "head": { "sha": head } }])).into_response()
}

async fn git_receive_pack(
    State(github): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    assert!(headers.contains_key("authorization"));
    let length = usize::from_str_radix(std::str::from_utf8(&body[..4]).unwrap(), 16).unwrap();
    let command = std::str::from_utf8(&body[4..length]).unwrap();
    let (command, _) = command.split_once('\0').unwrap();
    let mut parts = command.split(' ');
    let (from, to, name) = (
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
    );
    let mut github = github.lock().unwrap();
    let line = if github.branch == from {
        github.branch = to.to_string();
        github.updates.push((from.to_string(), to.to_string()));
        if github.processes_pushes {
            github.pull_head = to.to_string();
        }
        if let Some(racer) = github.racer.take() {
            github.branch = racer;
        }
        if std::mem::take(&mut github.lose_first_answer) {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        format!("ok {name}\n")
    } else {
        format!("ng {name} stale info\n")
    };
    let mut out = Vec::new();
    for payload in ["unpack ok\n", &line] {
        out.extend_from_slice(format!("{:04x}{payload}", payload.len() + 4).as_bytes());
    }
    out.extend_from_slice(b"0000");
    (StatusCode::OK, out).into_response()
}

async fn serve(github: Shared) -> String {
    let app = Router::new()
        .route(
            "/repos/soaplabs/soap-backend/git/ref/heads/feat/x",
            get(branch_ref),
        )
        .route("/repos/soaplabs/soap-backend/pulls", get(pulls))
        .route(
            "/soaplabs/soap-backend.git/git-receive-pack",
            post(git_receive_pack),
        )
        .with_state(github);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[expect(
    clippy::disallowed_methods,
    reason = "a test client against a local server; no `Config` timeouts apply"
)]
async fn watch_of(github: &Shared) -> (PushWatch, BranchMove) {
    let base = serve(github.clone()).await;
    let pushed = BranchMove {
        branch: "feat/x".to_string(),
        old: OLD.to_string(),
        new: NEW.to_string(),
    };
    let watch = PushWatch {
        http: reqwest::Client::new(),
        config: PushWatchConfig {
            api_base: format!("{base}/"),
            timing: Timing {
                settle: Duration::from_millis(1),
                bounce_gap: Duration::from_millis(1),
            },
        },
        git_url: format!("{base}/soaplabs/soap-backend.git"),
        owner: "soaplabs".to_string(),
        repo: "soap-backend".to_string(),
        username: "x-access-token",
        token: "app-token".to_string(),
        moves: vec![pushed.clone()],
        claim: None,
    };
    (watch, pushed)
}

fn github(pull_head: &str) -> Shared {
    Arc::new(Mutex::new(FakeGitHub {
        branch: NEW.to_string(),
        pull_head: pull_head.to_string(),
        ..FakeGitHub::default()
    }))
}

#[tokio::test]
async fn a_processed_push_is_left_alone() {
    let github = github(NEW);
    let (watch, pushed) = watch_of(&github).await;

    watch.watch(&pushed).await;

    assert!(github.lock().unwrap().updates.is_empty());
}

#[tokio::test]
async fn a_push_github_never_processed_is_bounced_until_the_pull_request_has_its_head() {
    let github = github(OLD);
    github.lock().unwrap().processes_pushes = true;
    let (watch, pushed) = watch_of(&github).await;

    watch.watch(&pushed).await;

    let github = github.lock().unwrap();
    assert_eq!(
        github.updates,
        vec![
            (NEW.to_string(), OLD.to_string()),
            (OLD.to_string(), NEW.to_string()),
        ]
    );
    assert_eq!(github.branch, NEW);
    assert_eq!(github.pull_head, NEW);
}

#[tokio::test]
async fn a_push_that_lands_during_a_repair_wins_it() {
    let github = github(OLD);
    github.lock().unwrap().racer = Some(RACER.to_string());
    let (watch, pushed) = watch_of(&github).await;

    watch.watch(&pushed).await;

    let github = github.lock().unwrap();
    assert_eq!(github.updates, vec![(NEW.to_string(), OLD.to_string())]);
    assert_eq!(
        github.branch, RACER,
        "the repair never overwrites a racing push"
    );
}

#[tokio::test]
async fn a_move_back_whose_answer_was_lost_still_completes_the_repair() {
    let github = github(OLD);
    {
        let mut github = github.lock().unwrap();
        github.processes_pushes = true;
        github.lose_first_answer = true;
    }
    let (watch, pushed) = watch_of(&github).await;

    watch.watch(&pushed).await;

    let github = github.lock().unwrap();
    assert_eq!(github.updates.len(), 2, "the forward move still ran");
    assert_eq!(github.branch, NEW, "the push is never left reverted");
    assert_eq!(github.pull_head, NEW);
}

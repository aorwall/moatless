//! In-process mock GitHub REST API for `moat gh` proxy E2E tests.
//!
//! Serves the endpoints the typed `/api/v1/github/*` backend routes call
//! (`GET /repos/{o}/{r}`, `POST /repos/{o}/{r}/pulls`,
//! `POST /repos/{o}/{r}/issues/{n}/comments`,
//! `POST /repos/{o}/{r}/pulls/{n}/comments/{cid}/replies`) and captures every
//! request for assertions. The backend is pointed at it via
//! `GITHUB_API_BASE_URL`.
//!
//! Same raw-TCP approach as [`crate::callback_server`]; each connection
//! serves one request and is closed (`Connection: close`) so reqwest never
//! reuses a dead socket.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A captured request to the mock GitHub API.
#[derive(Clone, Debug)]
pub struct CapturedGithubRequest {
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
}

/// Mock GitHub API server.
pub struct MockGithub {
    pub port: u16,
    captures: Arc<Mutex<Vec<CapturedGithubRequest>>>,
    handle: JoinHandle<()>,
}

/// PR number returned by the mock for `POST /repos/{o}/{r}/pulls`.
pub const MOCK_PR_NUMBER: u64 = 4242;
/// Comment id returned by the mock for `POST .../issues/{n}/comments`.
pub const MOCK_COMMENT_ID: u64 = 987_654;
/// Review comment id returned by the mock for
/// `POST .../pulls/{n}/comments/{cid}/replies`.
pub const MOCK_REPLY_ID: u64 = 111_222;
/// Default branch returned by the mock for `GET /repos/{o}/{r}`.
pub const MOCK_DEFAULT_BRANCH: &str = "main";

impl MockGithub {
    /// Start the mock on `port` (0 = ephemeral; the backend needs a fixed,
    /// pre-agreed port since `GITHUB_API_BASE_URL` is set at container start).
    pub async fn start(port: u16) -> Self {
        let listener = TcpListener::bind(("0.0.0.0", port))
            .await
            .unwrap_or_else(|e| panic!("mock github failed to bind port {port}: {e}"));
        let port = listener.local_addr().unwrap().port();
        let captures: Arc<Mutex<Vec<CapturedGithubRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captures_clone = Arc::clone(&captures);

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let captures = Arc::clone(&captures_clone);
                tokio::spawn(async move {
                    let _ = handle_connection(stream, captures).await;
                });
            }
        });

        Self {
            port,
            captures,
            handle,
        }
    }

    /// All captured requests so far.
    pub fn captures(&self) -> Vec<CapturedGithubRequest> {
        self.captures.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Wait until `predicate` matches a captured request, up to `timeout`.
    pub async fn wait_for(
        &self,
        timeout: Duration,
        predicate: impl Fn(&CapturedGithubRequest) -> bool,
    ) -> Option<CapturedGithubRequest> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(found) = self.captures().into_iter().find(|c| predicate(c)) {
                return Some(found);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Read one HTTP request (headers + Content-Length body), capture it, and
/// answer with the canned GitHub-shaped response for the route.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    captures: Arc<Mutex<Vec<CapturedGithubRequest>>>,
) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];

    // Read until we have the full head and Content-Length worth of body.
    let (head_end, content_length) = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_double_crlf(&buf) {
            let head = String::from_utf8_lossy(&buf[..pos]);
            let len = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            break (pos + 4, len);
        }
    };
    while buf.len() < head_end + content_length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let head = String::from_utf8_lossy(&buf[..head_end]);
    let mut request_line = head.lines().next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let body: serde_json::Value =
        serde_json::from_slice(&buf[head_end..]).unwrap_or(serde_json::Value::Null);

    let (status, payload) = route(&method, &path);

    if let Ok(mut vec) = captures.lock() {
        vec.push(CapturedGithubRequest { method, path, body });
    }

    let body_text = payload.to_string();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_text}",
        body_text.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Canned GitHub-shaped responses for the routes the typed endpoints hit.
fn route(method: &str, path: &str) -> (&'static str, serde_json::Value) {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        // POST /repos/{owner}/{repo}/pulls
        ("POST", ["repos", owner, repo, "pulls"]) => (
            "201 Created",
            serde_json::json!({
                "number": MOCK_PR_NUMBER,
                "html_url": format!("https://github.com/{owner}/{repo}/pull/{MOCK_PR_NUMBER}"),
                "title": "mock pr",
                "state": "open",
            }),
        ),
        // POST /repos/{owner}/{repo}/issues/{n}/comments
        ("POST", ["repos", _, _, "issues", _, "comments"]) => {
            ("201 Created", serde_json::json!({ "id": MOCK_COMMENT_ID }))
        }
        // POST /repos/{owner}/{repo}/pulls/{n}/comments/{cid}/replies
        ("POST", ["repos", _, _, "pulls", _, "comments", _, "replies"]) => {
            ("201 Created", serde_json::json!({ "id": MOCK_REPLY_ID }))
        }
        // GET /repos/{owner}/{repo} (default-branch lookup) and
        // GET /repos/{owner}/{repo}/pulls/{n} (PR fetch in hooks/runtime)
        ("GET", ["repos", ..]) => (
            "200 OK",
            serde_json::json!({
                "number": MOCK_PR_NUMBER,
                "title": "mock pr",
                "body": "",
                "state": "open",
                "default_branch": MOCK_DEFAULT_BRANCH,
                "head": { "ref": "feat/mock", "sha": "deadbeef" },
                "base": { "ref": MOCK_DEFAULT_BRANCH, "sha": "cafebabe" },
            }),
        ),
        _ => (
            "404 Not Found",
            serde_json::json!({ "message": "mock github: no route" }),
        ),
    }
}

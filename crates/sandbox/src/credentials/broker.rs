//! Unix-socket HTTP broker for sandbox-side credential pulls.
//!
//! Wired into git via `docker/moatless-git-cred.sh`; pulls the freshest value
//! from the backend on every request (no caching), so a rotation is visible to
//! the next `git push` without waiting for the credential refresher's cycle.
//! The backend's own host is the exception, answered from the pod's task token.
//! `GET /credential?key=K` returns `value\n`; `GET /git-credential?host=H`
//! returns git's credential-helper format.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::backend_client::BackendClient;

/// The git credential helper script answering from this broker
/// (`docker/moatless-git-cred.sh`), shipped in the sandbox image.
pub const BROKER_HELPER_PATH: &str = "/usr/local/bin/moatless-git-cred";

/// Shared state for the broker handlers.
#[derive(Clone)]
pub struct BrokerState {
    pub task_id: String,
    pub backend_client: Arc<dyn BackendClient>,
    /// `host[:port]` of the backend, which is the origin a proxied checkout
    /// clones from. A request naming it is answered with the pod's own task
    /// token instead of a provider token — see [`get_git_credential`].
    pub backend_host: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CredentialQuery {
    key: String,
}

async fn get_credential(
    State(state): State<BrokerState>,
    Query(q): Query<CredentialQuery>,
) -> impl IntoResponse {
    match state.backend_client.get_credentials(&state.task_id).await {
        Ok(resp) => {
            if let Some(value) = resp.credentials.get(&q.key) {
                (StatusCode::OK, format!("{value}\n"))
            } else {
                warn!(key = %q.key, "broker: the backend returned no such credential");
                (
                    StatusCode::NOT_FOUND,
                    format!("no credential for {}\n", q.key),
                )
            }
        }
        Err(e) => {
            warn!(error = %e, key = %q.key, "broker: get_credentials failed");
            (
                StatusCode::BAD_GATEWAY,
                format!("backend get_credentials failed: {e}\n"),
            )
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GitCredentialQuery {
    /// Git's own `host=` field, forwarded by the helper script. Absent when an
    /// older script asks, which only ever asked about the pod's git host.
    #[serde(default)]
    host: Option<String>,
}

/// Answer git's credential-helper protocol for one host.
///
/// A request naming the backend is a proxied checkout, answered with the task
/// token the pod already holds, with no backend round trip, so a slow
/// `get_credentials` cannot outlast the helper's timeout and leave git with no
/// credential. Every other host gets `GITHUB_TOKEN`, fetched fresh.
async fn get_git_credential(
    State(state): State<BrokerState>,
    Query(q): Query<GitCredentialQuery>,
) -> impl IntoResponse {
    let proxied = match (&state.backend_host, &q.host) {
        (Some(backend), Some(asked)) => backend == asked,
        _ => false,
    };
    if proxied {
        // The proxy ignores the username; the token is the whole credential.
        let Some(token) = state.backend_client.auth_token() else {
            warn!(host = ?q.host, "broker: no task token held for the backend git proxy");
            return (StatusCode::NOT_FOUND, "no task token held\n".to_string());
        };
        return (
            StatusCode::OK,
            format!("username=x-access-token\npassword={token}\n"),
        );
    }
    match state.backend_client.get_credentials(&state.task_id).await {
        Ok(resp) => {
            if let Some(token) = resp.credentials.get("GITHUB_TOKEN") {
                (
                    StatusCode::OK,
                    format!("username=oauth2\npassword={token}\n"),
                )
            } else {
                warn!(host = ?q.host, "broker: the backend returned no GITHUB_TOKEN for git");
                (
                    StatusCode::NOT_FOUND,
                    "no GITHUB_TOKEN available\n".to_string(),
                )
            }
        }
        Err(e) => {
            warn!(error = %e, host = ?q.host, "broker: get_credentials failed for git-credential");
            (
                StatusCode::BAD_GATEWAY,
                format!("backend get_credentials failed: {e}\n"),
            )
        }
    }
}

/// Build the broker router. Exposed for tests; production callers go through
/// [`spawn`].
pub fn router(state: BrokerState) -> Router {
    Router::new()
        .route("/credential", get(get_credential))
        .route("/git-credential", get(get_git_credential))
        .with_state(state)
}

/// Spawn the broker bound to a Unix domain socket at `socket_path`.
#[cfg(unix)]
pub fn spawn(
    state: BrokerState,
    socket_path: &Path,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>, String> {
    serve_on_socket(router(state), socket_path, cancel, "Credential broker")
}

/// Serve `app` on a Unix domain socket at `socket_path` until `cancel` fires.
///
/// Sets the socket mode to `0o666` so a non-root client can connect — safe
/// because the socket lives in the per-pod filesystem namespace.
#[cfg(unix)]
pub fn serve_on_socket(
    app: Router,
    socket_path: &Path,
    cancel: CancellationToken,
    name: &'static str,
) -> Result<tokio::task::JoinHandle<()>, String> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    if let Some(parent) = socket_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("create_dir_all({}): {e}", parent.display()));
        }
    }
    // Best-effort cleanup of stale socket from a previous run.
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("UnixListener::bind({}): {e}", socket_path.display()))?;
    if let Err(e) = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666)) {
        warn!(error = %e, socket = %socket_path.display(), name, "Failed to chmod a unix socket");
    }
    let socket_path = socket_path.to_path_buf();

    Ok(tokio::spawn(async move {
        info!(socket = %socket_path.display(), name, "Unix socket server started");
        let serve = axum::serve(listener, app.into_make_service());
        tokio::select! {
            res = serve => {
                if let Err(e) = res {
                    warn!(error = %e, name, "Unix socket server exited with error");
                }
            }
            () = cancel.cancelled() => {
                info!(name, "Unix socket server shutting down");
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::http::StatusCode as Sc;
    use sandbox_proto::GetCredentialsResponse;
    use std::collections::HashMap;

    struct StaticBackend {
        creds: HashMap<String, String>,
        task_token: Option<String>,
        fetches: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl BackendClient for StaticBackend {
        async fn get_credentials(&self, _: &str) -> Result<GetCredentialsResponse, tonic::Status> {
            self.fetches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(GetCredentialsResponse {
                credentials: self.creds.clone(),
            })
        }

        fn auth_token(&self) -> Option<String> {
            self.task_token.clone()
        }
    }

    fn make_state(creds: HashMap<String, String>) -> BrokerState {
        state_over(Arc::new(StaticBackend {
            creds,
            task_token: Some("task-jwt".to_string()),
            fetches: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    fn state_over(backend: Arc<StaticBackend>) -> BrokerState {
        let backend: Arc<dyn BackendClient> = backend;
        BrokerState {
            task_id: "task-1".to_string(),
            backend_client: backend,
            backend_host: Some("backend:8001".to_string()),
        }
    }

    #[tokio::test]
    async fn returns_credential_value() {
        let mut creds = HashMap::new();
        creds.insert("GITHUB_TOKEN".to_string(), "ghs_xxx".to_string());
        let state = make_state(creds);

        let resp = get_credential(
            State(state),
            Query(CredentialQuery {
                key: "GITHUB_TOKEN".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), Sc::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), b"ghs_xxx\n");
    }

    #[tokio::test]
    async fn returns_404_for_missing_credential() {
        let state = make_state(HashMap::new());

        let resp = get_credential(
            State(state),
            Query(CredentialQuery {
                key: "NOPE".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(resp.status(), Sc::NOT_FOUND);
    }

    async fn git_credential_body(state: BrokerState, host: Option<&str>) -> (Sc, String) {
        let resp = get_git_credential(
            State(state),
            Query(GitCredentialQuery {
                host: host.map(str::to_string),
            }),
        )
        .await
        .into_response();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn git_credential_returns_helper_format() {
        let mut creds = HashMap::new();
        creds.insert("GITHUB_TOKEN".to_string(), "ghs_token".to_string());

        let (status, body) = git_credential_body(make_state(creds), Some("github.com")).await;
        assert_eq!(status, Sc::OK);
        assert!(body.contains("username=oauth2"));
        assert!(body.contains("password=ghs_token"));
    }

    /// A proxied checkout addresses the backend, and the credential it needs is
    /// the task token the pod already holds — never the git host's, and with no
    /// backend call.
    #[tokio::test]
    async fn the_backend_host_is_answered_with_the_task_token() {
        let mut creds = HashMap::new();
        creds.insert("GITHUB_TOKEN".to_string(), "ghs_token".to_string());
        let backend = Arc::new(StaticBackend {
            creds: creds.clone(),
            task_token: Some("task-jwt".to_string()),
            fetches: std::sync::atomic::AtomicUsize::new(0),
        });

        let (status, body) =
            git_credential_body(state_over(Arc::clone(&backend)), Some("backend:8001")).await;
        assert_eq!(status, Sc::OK);
        assert!(body.contains("password=task-jwt"), "{body}");
        assert!(!body.contains("ghs_token"), "{body}");
        assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 0);

        // A host the helper did not name is not the backend.
        let (_, body) = git_credential_body(make_state(creds), None).await;
        assert!(body.contains("password=ghs_token"), "{body}");
    }

    #[tokio::test]
    async fn the_backend_host_with_no_task_token_held_answers_404() {
        let backend = Arc::new(StaticBackend {
            creds: HashMap::new(),
            task_token: None,
            fetches: std::sync::atomic::AtomicUsize::new(0),
        });
        let (status, _) = git_credential_body(state_over(backend), Some("backend:8001")).await;
        assert_eq!(status, Sc::NOT_FOUND);
    }

    #[test]
    fn router_compiles_for_unix_socket_serve() {
        // Smoke-test that the router builder + state shape line up. Real
        // socket-bound serving is exercised by `spawn_serves_git_credential_over_socket`
        // below.
        let _ = router(make_state(HashMap::new()));
    }

    /// End-to-end: spawn the broker on a real unix socket, send the same
    /// HTTP/1.1 GET `curl --unix-socket` sends, and assert the response matches
    /// the git credential-helper protocol.
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_serves_git_credential_over_socket() {
        use std::os::unix::fs::PermissionsExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("credentials.sock");

        let mut creds = HashMap::new();
        creds.insert("GITHUB_TOKEN".to_string(), "ghu_test_token".to_string());
        let state = make_state(creds);
        let cancel = CancellationToken::new();

        super::spawn(state, &socket_path, cancel.clone()).expect("broker spawn failed");

        // Give the listener a tick to start accepting.
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket_path.exists(), "broker socket was not created");

        // Socket must be world-rw so non-root credential helpers can connect
        // if we ever drop sandbox privileges.
        let mode = std::fs::metadata(&socket_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o666,
            "socket perms should be 0666, got {mode:o}"
        );

        let mut stream = tokio::net::UnixStream::connect(&socket_path)
            .await
            .expect("connect to broker socket");
        stream
            .write_all(
                b"GET /git-credential HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.starts_with("HTTP/1.1 200"),
            "expected 200, got: {resp}"
        );
        assert!(
            resp.contains("username=oauth2"),
            "missing username line: {resp}"
        );
        assert!(
            resp.contains("password=ghu_test_token"),
            "missing password line: {resp}"
        );

        cancel.cancel();
    }
}

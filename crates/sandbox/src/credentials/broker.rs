//! Unix-socket HTTP broker for sandbox-side git credentials.
//!
//! Wired into git via `docker/moatless-git-cred.sh`. It answers only the
//! backend's own host, which every git remote reaches (directly, or through
//! `git-remote-git+https` for github.com), with the pod's task token. No git
//! host's own token is held in the pod. `GET /git-credential?host=H` returns
//! git's credential-helper format.

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
    /// Holds the task token, swapped in place when the backend rotates it.
    pub backend_client: Arc<dyn BackendClient>,
    /// `host[:port]` of the backend, the one host this broker answers.
    pub backend_host: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct GitCredentialQuery {
    /// Git's own `host=` field, forwarded by the helper script.
    #[serde(default)]
    host: Option<String>,
}

/// Answer git's credential-helper protocol for one host: the backend with the
/// task token the pod already holds, with no backend round trip; any other
/// host with 404, so git falls through to whatever else is configured.
async fn get_git_credential(
    State(state): State<BrokerState>,
    Query(q): Query<GitCredentialQuery>,
) -> impl IntoResponse {
    let proxied = match (&state.backend_host, &q.host) {
        (Some(backend), Some(asked)) => backend == asked,
        _ => false,
    };
    if !proxied {
        return (
            StatusCode::NOT_FOUND,
            "the broker answers only the backend's git proxy\n".to_string(),
        );
    }
    // The proxy ignores the username; the token is the whole credential.
    let Some(token) = state.backend_client.auth_token() else {
        warn!(host = ?q.host, "broker: no task token held for the backend git proxy");
        return (StatusCode::NOT_FOUND, "no task token held\n".to_string());
    };
    (
        StatusCode::OK,
        format!("username=x-access-token\npassword={token}\n"),
    )
}

/// Build the broker router. Exposed for tests; production callers go through
/// [`spawn`].
pub fn router(state: BrokerState) -> Router {
    Router::new()
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

    /// A backend holding `task_token` and a `GITHUB_TOKEN` credential, which
    /// the broker must never hand out.
    struct StaticBackend {
        task_token: Option<String>,
        fetches: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl BackendClient for StaticBackend {
        async fn get_credentials(&self, _: &str) -> Result<GetCredentialsResponse, tonic::Status> {
            self.fetches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(GetCredentialsResponse {
                credentials: HashMap::from([("GITHUB_TOKEN".to_string(), "ghs_token".to_string())]),
                ..Default::default()
            })
        }

        fn auth_token(&self) -> Option<String> {
            self.task_token.clone()
        }
    }

    fn backend(task_token: Option<&str>) -> Arc<StaticBackend> {
        Arc::new(StaticBackend {
            task_token: task_token.map(str::to_string),
            fetches: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn state_over(backend: Arc<StaticBackend>) -> BrokerState {
        let backend: Arc<dyn BackendClient> = backend;
        BrokerState {
            backend_client: backend,
            backend_host: Some("backend:8001".to_string()),
        }
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

    /// Every git remote reaches the backend, and the credential it needs is
    /// the task token the pod already holds, with no backend call.
    #[tokio::test]
    async fn the_backend_host_is_answered_with_the_task_token() {
        let backend = backend(Some("task-jwt"));

        let (status, body) =
            git_credential_body(state_over(Arc::clone(&backend)), Some("backend:8001")).await;
        assert_eq!(status, Sc::OK);
        assert!(body.contains("username=x-access-token"), "{body}");
        assert!(body.contains("password=task-jwt"), "{body}");
        assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn github_or_an_unnamed_host_gets_no_credential() {
        for host in [Some("github.com"), None] {
            let backend = backend(Some("task-jwt"));
            let (status, body) = git_credential_body(state_over(Arc::clone(&backend)), host).await;
            assert_eq!(status, Sc::NOT_FOUND, "{host:?}");
            assert!(
                !body.contains("ghs_token") && !body.contains("task-jwt"),
                "{body}"
            );
            assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn the_backend_host_with_no_task_token_held_answers_404() {
        let (status, _) =
            git_credential_body(state_over(backend(None)), Some("backend:8001")).await;
        assert_eq!(status, Sc::NOT_FOUND);
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
        let cancel = CancellationToken::new();

        super::spawn(
            state_over(backend(Some("task-jwt"))),
            &socket_path,
            cancel.clone(),
        )
        .expect("broker spawn failed");

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
                b"GET /git-credential?host=backend:8001 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
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
            resp.contains("password=task-jwt"),
            "missing password line: {resp}"
        );

        cancel.cancel();
    }
}

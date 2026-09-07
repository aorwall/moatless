//! Unix-socket HTTP broker for sandbox-side credential pulls.
//!
//! Wired into git via the `/usr/local/bin/moatless-git-cred` helper script
//! (see `docker/moatless-git-cred.sh`) which `init/git.rs` registers as
//! `credential.https://github.com.helper`. The broker pulls the freshest value
//! from the backend on every request — no caching — so a backend-side rotation
//! is visible to the next `git push` without waiting for the 15-min hosts.yml
//! refresher cycle.
//!
//! # Endpoints
//!
//! - `GET /credential?key=GITHUB_TOKEN` → plain `value\n`
//! - `GET /git-credential` → git's credential-helper format
//!   (`username=oauth2\npassword=<token>\n`)

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

/// Shared state for the broker handlers.
#[derive(Clone)]
pub struct BrokerState {
    pub task_id: String,
    pub backend_client: Arc<dyn BackendClient>,
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
        Ok(resp) => match resp.credentials.get(&q.key) {
            Some(value) => (StatusCode::OK, format!("{value}\n")),
            None => (
                StatusCode::NOT_FOUND,
                format!("no credential for {}\n", q.key),
            ),
        },
        Err(e) => {
            warn!(error = %e, key = %q.key, "broker: get_credentials failed");
            (
                StatusCode::BAD_GATEWAY,
                format!("backend get_credentials failed: {e}\n"),
            )
        }
    }
}

async fn get_git_credential(State(state): State<BrokerState>) -> impl IntoResponse {
    match state.backend_client.get_credentials(&state.task_id).await {
        Ok(resp) => match resp.credentials.get("GITHUB_TOKEN") {
            Some(token) => (
                StatusCode::OK,
                format!("username=oauth2\npassword={token}\n"),
            ),
            None => (
                StatusCode::NOT_FOUND,
                "no GITHUB_TOKEN available\n".to_string(),
            ),
        },
        Err(e) => {
            warn!(error = %e, "broker: get_credentials failed for git-credential");
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
///
/// Sets the socket mode to `0o666` so credential helpers running as a non-root
/// user (when the sandbox image is hardened to drop root) can still connect.
/// This is acceptable because the socket lives in the per-pod filesystem
/// namespace — there are no other users with shell access to the pod.
#[cfg(unix)]
pub fn spawn(
    state: BrokerState,
    socket_path: &Path,
    cancel: CancellationToken,
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
        warn!(error = %e, socket = %socket_path.display(), "Failed to chmod credential broker socket");
    }
    let app = router(state);
    let socket_path = socket_path.to_path_buf();

    Ok(tokio::spawn(async move {
        info!(socket = %socket_path.display(), "Credential broker started");
        let serve = axum::serve(listener, app.into_make_service());
        tokio::select! {
            res = serve => {
                if let Err(e) = res {
                    warn!(error = %e, "Credential broker exited with error");
                }
            }
            () = cancel.cancelled() => {
                info!("Credential broker shutting down");
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::http::StatusCode as Sc;
    use sandbox_proto::{GetCredentialsResponse, PendingAgentDelivery, PublishEventRequest};
    use std::collections::HashMap;

    struct StaticBackend {
        creds: HashMap<String, String>,
    }

    #[async_trait]
    impl BackendClient for StaticBackend {
        async fn publish_event(&self, _: PublishEventRequest) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_pending_messages(
            &self,
            _: &str,
        ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
            Ok(vec![])
        }
        async fn acknowledge_messages(
            &self,
            _: &str,
            _: Vec<String>,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_credentials(&self, _: &str) -> Result<GetCredentialsResponse, tonic::Status> {
            Ok(GetCredentialsResponse {
                credentials: self.creds.clone(),
                server_env: HashMap::new(),
                ssh_keys: HashMap::new(),
            })
        }
        async fn update_credential(&self, _: &str, _: &str, _: &str) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_system_prompt(&self, _: &str) -> Result<String, tonic::Status> {
            Ok(String::new())
        }
        async fn download_upload(
            &self,
            _: &str,
            _: &str,
        ) -> Result<sandbox_proto::DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }
        async fn recover_interrupted_run(
            &self,
            _: &str,
        ) -> Result<Option<sandbox_proto::InterruptedTurn>, tonic::Status> {
            Ok(None)
        }

        async fn upload_transcript(&self, _: &str, _: Vec<u8>) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn download_transcript(&self, _: &str) -> Result<Vec<u8>, tonic::Status> {
            Ok(Vec::new())
        }
    }

    fn make_state(creds: HashMap<String, String>) -> BrokerState {
        let backend: Arc<dyn BackendClient> = Arc::new(StaticBackend { creds });
        BrokerState {
            task_id: "task-1".to_string(),
            backend_client: backend,
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

    #[tokio::test]
    async fn git_credential_returns_helper_format() {
        let mut creds = HashMap::new();
        creds.insert("GITHUB_TOKEN".to_string(), "ghs_token".to_string());
        let state = make_state(creds);

        let resp = get_git_credential(State(state)).await.into_response();
        assert_eq!(resp.status(), Sc::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("username=oauth2"));
        assert!(body_str.contains("password=ghs_token"));
    }

    #[test]
    fn router_compiles_for_unix_socket_serve() {
        // Smoke-test that the router builder + state shape line up. Real
        // socket-bound serving is exercised by `spawn_serves_git_credential_over_socket`
        // below.
        let _ = router(make_state(HashMap::new()));
    }

    /// End-to-end: spawn the broker on a real unix socket, do the minimal
    /// HTTP/1.1 GET that mirrors what the `moatless-git-cred` helper sends
    /// (via `curl --unix-socket`), and assert the response body matches the
    /// git credential-helper protocol shape.
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

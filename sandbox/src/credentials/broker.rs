//! Unix-socket HTTP broker for sandbox-side credential pulls.
//!
//! Tools that support credential helpers (`git`, `gh`) can be configured to
//! call a wrapper script that hits this broker. The broker pulls the freshest
//! value from the backend on every request — no caching — so a backend-side
//! rotation is visible to the next `git push` without any push notification.
//!
//! # Endpoints
//!
//! - `GET /credential?key=GITHUB_TOKEN` → plain `value\n`
//! - `GET /git-credential` → git's credential-helper format
//!   (`username=oauth2\npassword=<token>\n`)
//!
//! # Integration (TODO follow-up)
//!
//! The shell wrappers + git config aren't wired in this iteration:
//!
//! 1. `/usr/local/bin/moatless-git-cred` shell script that issues
//!    `curl --unix-socket /run/moatless/credentials.sock http://./git-credential`.
//! 2. `git config --global credential.helper '!/usr/local/bin/moatless-git-cred'`
//!    at sandbox init.
//! 3. Optional: `gh auth setup-git` integration.
//!
//! Until wired, the broker compiles and is unit-tested but is opt-in via a
//! call to [`spawn`] from `main.rs` (currently disabled by default).

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
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
/// Currently behind a feature gate at the call-site — wire-up of the git/gh
/// helper scripts that consume this socket is pending.
#[cfg(unix)]
pub fn spawn(
    state: BrokerState,
    socket_path: &Path,
    cancel: CancellationToken,
) -> Result<(), String> {
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
    let app = router(state);
    let socket_path = socket_path.to_path_buf();

    tokio::spawn(async move {
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
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::http::StatusCode as Sc;
    use sandbox_proto::{GetCredentialsResponse, PendingMessage, PublishEventRequest};
    use std::collections::HashMap;

    struct StaticBackend {
        creds: HashMap<String, String>,
    }

    #[async_trait]
    impl BackendClient for StaticBackend {
        async fn publish_event(&self, _: PublishEventRequest) {}
        async fn get_pending_messages(
            &self,
            _: &str,
        ) -> Result<Vec<PendingMessage>, tonic::Status> {
            Ok(vec![])
        }
        async fn acknowledge_messages(
            &self,
            _: &str,
            _: Vec<String>,
            _: &str,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_credentials(&self, _: &str) -> Result<GetCredentialsResponse, tonic::Status> {
            Ok(GetCredentialsResponse {
                credentials: self.creds.clone(),
                server_env: HashMap::new(),
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
        // socket-bound serving is exercised in integration tests when the
        // broker is wired to git/gh helper scripts.
        let _ = router(make_state(HashMap::new()));
    }
}

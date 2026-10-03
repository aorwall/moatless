//! The socket as `gh` drives it, against a backend that records what arrives.

use std::sync::Mutex;

use async_trait::async_trait;
use axum::http::HeaderMap;
use sandbox_proto::{GetCredentialsResponse, PendingAgentDelivery, PublishEventRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::credentials::gh_config::PLACEHOLDER_TOKEN;

/// Holds `token` as its task token and answers nothing else.
struct TokenBackend {
    token: Option<String>,
}

#[async_trait]
impl BackendClient for TokenBackend {
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
        Ok(GetCredentialsResponse::default())
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
    fn auth_token(&self) -> Option<String> {
        self.token.clone()
    }
}

/// One request as the fake backend saw it.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    authorization: Option<String>,
    body: String,
}

/// A backend that records every request and answers `{"login":"bot"}`.
async fn recording_backend() -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let app = Router::new().fallback(
        move |uri: axum::http::Uri, headers: HeaderMap, body: String| {
            let recorded = Arc::clone(&recorded);
            async move {
                recorded.lock().unwrap().push(Seen {
                    path: uri.to_string(),
                    authorization: headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    body,
                });
                axum::Json(serde_json::json!({ "login": "bot" }))
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

async fn spawn_socket(
    backend_url: Option<&str>,
    task_token: Option<&str>,
) -> (tempfile::TempDir, std::path::PathBuf, CancellationToken) {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("github.sock");
    let state = GitHubProxyState::new(
        Arc::new(TokenBackend {
            token: task_token.map(str::to_string),
        }),
        backend_url,
        crate::http_config::SandboxHttpConfig::default().github,
    );
    let cancel = CancellationToken::new();
    crate::credentials::broker::serve_on_socket(router(state), &socket, cancel.clone(), "test")
        .unwrap();
    (tmp, socket, cancel)
}

/// Send `request` over `socket` as `gh` does with `http_unix_socket` set:
/// plain HTTP/1.1, with the host it meant in `Host`.
async fn send(socket: &std::path::Path, request: &str) -> String {
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

/// What `gh api user` writes to the socket, placeholder token and all.
fn gh_request(method: &str, path: &str, body: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: api.github.com\r\nAuthorization: token {PLACEHOLDER_TOKEN}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The `gh api` call in a long `moat cmd run` process: whatever `gh` holds,
/// the backend receives the pod's task token, and decides the GitHub token
/// itself.
#[tokio::test]
async fn an_api_request_reaches_the_backend_with_the_task_token_in_place_of_gh_s() {
    let (backend, seen) = recording_backend().await;
    let (_tmp, socket, cancel) = spawn_socket(Some(&format!("{backend}/")), Some("task-jwt")).await;

    let response = send(
        &socket,
        &gh_request("GET", "/repos/o/r/pulls?state=open", ""),
    )
    .await;
    let posted = send(
        &socket,
        &gh_request("POST", "/graphql", r#"{"query":"{viewer{login}}"}"#),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#""login":"bot""#), "{response}");
    assert!(posted.starts_with("HTTP/1.1 200"), "{posted}");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0].path, "/github/api/repos/o/r/pulls?state=open");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer task-jwt"));
    assert_eq!(seen[1].path, "/github/api/graphql");
    assert_eq!(seen[1].body, r#"{"query":"{viewer{login}}"}"#);
    cancel.cancel();
}

#[tokio::test]
async fn a_sandbox_without_its_task_token_yet_says_so() {
    let (backend, seen) = recording_backend().await;
    let (_tmp, socket, cancel) = spawn_socket(Some(&backend), None).await;

    let response = send(&socket, &gh_request("GET", "/user", "")).await;

    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert!(response.contains("no task token"), "{response}");
    assert!(seen.lock().unwrap().is_empty());
    cancel.cancel();
}

#[test]
fn api_hosts_go_to_the_backend_and_every_other_host_goes_direct() {
    let backend = Some("http://backend:8001");

    assert_eq!(
        target("api.github.com", "/user", backend),
        Some(Target::Backend(
            "http://backend:8001/github/api/user".to_string()
        ))
    );
    assert_eq!(
        target(
            "uploads.github.com:443",
            "/repos/o/r/releases/1/assets?name=a",
            backend
        ),
        Some(Target::Backend(
            "http://backend:8001/github/uploads/repos/o/r/releases/1/assets?name=a".to_string()
        ))
    );
    // Where `gh run view --log` is redirected: signed, so no credential.
    assert_eq!(
        target("results.blob.core.windows.net", "/logs.zip?sig=1", backend),
        Some(Target::Direct(
            "https://results.blob.core.windows.net/logs.zip?sig=1".to_string()
        ))
    );
    assert_eq!(target("api.github.com", "/user", None), None);
}

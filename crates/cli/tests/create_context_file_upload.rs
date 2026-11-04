//! `moat tasks create --context-file` against a stub backend: the bytes are
//! uploaded, and the upload's id is what the create request attaches.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use moatless_cli::client::MoatClient;
use moatless_cli::commands::tasks::{TaskAction, run};
use moatless_cli::config::{AuthMode, Config, HttpConfig};

/// What the stub backend was asked to do, in call order.
#[derive(Default)]
struct Recorded {
    /// The raw multipart body of each `POST /api/v1/uploads`.
    uploads: Vec<String>,
    /// The JSON body of each `POST /api/v1/tasks`.
    creates: Vec<serde_json::Value>,
}

async fn upload(
    State(state): State<Arc<Mutex<Recorded>>>,
    body: String,
) -> axum::Json<serde_json::Value> {
    state.lock().expect("recorder").uploads.push(body);
    axum::Json(serde_json::json!({
        "fileId": "file-from-upload",
        "filename": "implementation-context.md",
        "mimeType": "text/markdown",
        "size": 7,
    }))
}

async fn create(
    State(state): State<Arc<Mutex<Recorded>>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    state.lock().expect("recorder").creates.push(body);
    axum::Json(serde_json::json!({
        "taskId": "task-1",
        "status": "open",
        "task": {
            "taskId": "task-1",
            "title": "t",
            "status": "open",
            "agentProfile": "claude-code",
            "repositoryId": "repo-1",
            "createdAt": "2026-01-01T00:00:00Z",
            "lastMessageAt": "2026-01-01T00:00:00Z",
            "totalCostUsd": 0.0,
            "visibility": "private",
            "scope": "user",
            "tags": [],
            "ttlMinutes": 60,
            "closeAfterSettledDays": 7,
            "sandboxDesiredState": "running",
            "sandboxDesiredStateUpdatedAt": "2026-01-01T00:00:00Z",
        }
    }))
}

/// Serve the two endpoints a `--workspace ws_…` create touches, and return the
/// base url plus the recorder. A `ws_`-prefixed workspace resolves without an
/// HTTP call, so nothing else is reached.
async fn stub_backend() -> (String, Arc<Mutex<Recorded>>) {
    let state = Arc::new(Mutex::new(Recorded::default()));
    let app = Router::new()
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/tasks", post(create))
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), state)
}

fn create_action(context_file: &str) -> TaskAction {
    TaskAction::Create {
        repo: None,
        workspace: Some("ws_test".to_string()),
        same_sandbox: None,
        checkout_mode: None,
        name: None,
        parent: None,
        run_as: None,
        message: Some("read the attached brief".to_string()),
        context_files: vec![context_file.to_string()],
        branch: None,
        agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
        model: None,
        effort: None,
        headless: false,
    }
}

#[tokio::test]
async fn a_context_file_is_uploaded_and_its_id_is_what_create_attaches() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("implementation-context.md");
    std::fs::write(&path, "# brief").expect("write the context file");

    let (base_url, recorded) = stub_backend().await;
    let client = MoatClient::new_with_api_key(&base_url, "test-key", HttpConfig::default());
    let config = Config {
        api_url: base_url,
        auth_mode: AuthMode::Standalone { api_key: None },
        login: moatless_cli::config::LoginEnv::default(),
        sandbox_grpc_port: "50052".to_string(),
        gh_repo: None,
        github_token_set: false,
        http: HttpConfig::default(),
    };

    run(
        &client,
        create_action(path.to_str().expect("utf-8 path")),
        true,
        &config,
    )
    .await
    .expect("create succeeds against the stub backend");

    let recorded = recorded.lock().expect("recorder");
    let [upload] = recorded.uploads.as_slice() else {
        panic!(
            "expected exactly one upload, got {}",
            recorded.uploads.len()
        );
    };
    assert!(
        upload.contains("implementation-context.md") && upload.contains("# brief"),
        "the file's name and bytes must reach the upload endpoint: {upload}"
    );

    let [create] = recorded.creates.as_slice() else {
        panic!(
            "expected exactly one create, got {}",
            recorded.creates.len()
        );
    };
    assert_eq!(
        create.get("fileIds"),
        Some(&serde_json::json!(["file-from-upload"]))
    );
    // A path here is what the child task reports as `File metadata not found`.
    assert_eq!(create.get("contexts"), Some(&serde_json::json!([])));
}

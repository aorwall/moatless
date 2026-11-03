use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{Json, Router, routing::post};
use sandbox_proto::{AppendTrajectoryRequest, TrajectoryLine};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::agent::harness::trajectory::{ClaudeSessionFormat, SessionFormat};
use crate::agent::harness::{CodexConfig, HarnessRuntime, Input, Outcome, Signal};
use crate::agent::types::ResumeSession;
use crate::backend_client::BackendClient;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::trajectory_tail::{TrajectoryRecording, memory::MemoryTrajectoryBackend};

fn model_response() -> String {
    let message = json!({
        "id": "msg-1", "type": "message", "role": "assistant", "status": "completed",
        "content": [{"type": "output_text", "text": "PERIWINKLE", "annotations": []}],
    });
    let mut stream = String::new();
    for event in [
        json!({"type": "response.created", "response": {"id": "resp-1"}}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": message}),
        json!({"type": "response.completed", "response": {
            "id": "resp-1", "status": "completed", "output": [message],
            "usage": {"input_tokens": 10, "output_tokens": 2, "total_tokens": 12},
        }}),
    ] {
        writeln!(
            stream,
            "event: {}\ndata: {event}\n",
            event["type"].as_str().unwrap()
        )
        .expect("write SSE frame");
    }
    stream
}

async fn completed_session(harness: &mut HarnessRuntime, input: Input) -> String {
    harness.submit(input).await.expect("submit");
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut session = None;
        let mut accepted = 0;
        loop {
            match harness.next_signal().await.expect("harness open") {
                Signal::ExecutionStarted { session: id, .. } => session = Some(id.0),
                Signal::InputAccepted { .. } => accepted += 1,
                Signal::InputRejected { reason, .. } => panic!("fallback rejected: {reason:?}"),
                Signal::ExecutionFinished { outcome } => {
                    assert!(matches!(outcome, Outcome::Completed { .. }), "{outcome:?}");
                    assert_eq!(accepted, 1);
                    return session.expect("session announced");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("turn completed")
}

#[tokio::test]
#[ignore = "requires Codex CLI; model requests go to an isolated local stub"]
async fn claude_fallback_continues_with_history_and_resumes_the_same_codex_session() {
    let home = tempfile::tempdir().expect("home");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let app = Router::new().route(
        "/responses",
        post(move |Json(body): Json<Value>| {
            let captured = Arc::clone(&captured);
            async move {
                captured.lock().expect("requests").push(body);
                ([("content-type", "text/event-stream")], model_response())
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .expect("local provider");
    });
    tokio::fs::write(
        home.path().join("config.toml"),
        format!(
            "model_provider = \"local_gateway\"\n[model_providers.local_gateway]\n\
         name = \"Test\"\nbase_url = \"http://{address}\"\n\
         wire_api = \"responses\"\nrequires_openai_auth = false\n",
        ),
    )
    .await
    .expect("config");
    let workspace_file = home.path().join("worktree.txt");
    tokio::fs::write(&workspace_file, "unfinished work")
        .await
        .expect("workspace");
    let mut config = crate::config::test_support::minimal_config();
    config.codex_cli_path = std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH")
        .unwrap_or_else(|_| "/usr/local/bin/codex".to_string());
    config.workspace_dir = home.path().display().to_string();
    config.repo_dir = config.workspace_dir.clone();
    config.home_dir = config.workspace_dir.clone();
    config.codex_home_dir = config.workspace_dir.clone();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let payload = json!({
        "type": "user", "sessionId": "claude-session",
        "message": {"role": "user", "content": "Remember the word PERIWINKLE"},
    })
    .to_string();
    let read = ClaudeSessionFormat.read_line(&payload);
    backend
        .append_trajectory(AppendTrajectoryRequest {
            task_id: config.task_id.clone(),
            harness: "claude-code".to_string(),
            harness_session_id: "claude-session".to_string(),
            first_seq: 0,
            lines: vec![TrajectoryLine {
                kind: read.kind,
                content_json: read.content.to_json(),
                payload,
                ..TrajectoryLine::default()
            }],
            ..AppendTrajectoryRequest::default()
        })
        .await
        .expect("record Claude history");
    let mut harness = HarnessRuntime::codex(CodexConfig {
        config: Arc::new(config),
        credential_failures: CredentialFailureDetector::default(),
        trajectory: Some(TrajectoryRecording::new(backend)),
    });
    let first = completed_session(
        &mut harness,
        Input {
            model: Some("gpt-test".to_string()),
            switch_harness: true,
            start_fresh: true,
            ..Input::text("fallback", "Continue and repeat the remembered word")
        },
    )
    .await;
    let second = completed_session(
        &mut harness,
        Input {
            model: Some("gpt-test".to_string()),
            resume: Some(ResumeSession::WithId(first.clone())),
            ..Input::text("follow-up", "Repeat it again")
        },
    )
    .await;
    assert_eq!(first, second);
    assert_eq!(
        tokio::fs::read_to_string(workspace_file)
            .await
            .expect("workspace"),
        "unfinished work"
    );
    let captured = requests.lock().expect("requests").clone();
    assert_eq!(captured.len(), 2);
    for body in captured {
        assert!(
            body["input"]
                .to_string()
                .contains("Remember the word PERIWINKLE"),
            "history missing: {body}"
        );
    }
    drop(harness);
    stop.cancel();
    server.await.expect("provider stopped");
}

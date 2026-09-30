//! `moat decide pick --each` against a stub backend: rows come back in input
//! order however long each decision takes, and a failed item does not stop
//! the batch.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures_util::StreamExt;
use moatless_api_schemas::decisions::{DecisionOption, PickDecisionRequest};
use moatless_cli::client::MoatClient;
use moatless_cli::commands::decide::{BatchItem, DecideAction, PickArgs, pick_each, run};
use moatless_cli::config::{AuthMode, Config, HttpConfig};
use serde_json::{Value, json};

/// Answers with the state as the choice; `slow` answers late, `fail` is refused.
async fn pick(
    State(recorded): State<Arc<Mutex<Vec<Value>>>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    recorded.lock().expect("recorder").push(body.clone());
    let state = body["state"].as_str().unwrap_or_default().to_string();
    if state == "fail" {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "state rejected"})),
        )
            .into_response();
    }
    if state == "slow" {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    axum::Json(json!({
        "choice": state,
        "confidence": 0.9,
        "band": "act",
        "probabilities": [],
        "provider": "jev",
        "model": "jev-1.13.0",
        "usage": {"inputTokens": 10, "outputTokens": 2},
    }))
    .into_response()
}

async fn stub_backend() -> (String, Arc<Mutex<Vec<Value>>>) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/api/v1/decisions/pick", post(pick))
        .with_state(Arc::clone(&recorded));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), recorded)
}

fn item(id: &str, state: &str) -> BatchItem {
    BatchItem {
        id: id.to_string(),
        state: state.to_string(),
    }
}

#[tokio::test]
async fn each_keeps_input_order_and_reports_a_failed_item_in_place() {
    let (base_url, recorded) = stub_backend().await;
    let client = MoatClient::new_with_api_key(&base_url, "test-key", HttpConfig::default());
    let template = PickDecisionRequest {
        question: "Which?".to_string(),
        options: vec![DecisionOption {
            name: "slow".to_string(),
            description: Some("the slow one".to_string()),
        }],
        state: String::new(),
        task_id: Some("task-1".to_string()),
    };

    let results: Vec<_> = pick_each(
        client.api(),
        &template,
        vec![item("1", "slow"), item("2", "fail"), item("3", "fast")],
    )
    .collect()
    .await;

    let rows: Vec<(&str, Option<&str>)> = results
        .iter()
        .map(|(id, result)| (id.as_str(), result.as_ref().ok().map(|r| r.choice.as_str())))
        .collect();
    assert_eq!(
        rows,
        vec![("1", Some("slow")), ("2", None), ("3", Some("fast"))]
    );
    let recorded = recorded.lock().expect("recorder");
    assert_eq!(recorded.len(), 3);
    let first = recorded
        .iter()
        .find(|body| body["state"] == "slow")
        .expect("slow request");
    assert_eq!(
        *first,
        json!({
            "question": "Which?",
            "options": [{"name": "slow", "description": "the slow one"}],
            "state": "slow",
            "taskId": "task-1",
        })
    );
}

#[tokio::test]
async fn each_fails_the_command_after_every_row_when_an_item_failed() {
    let (base_url, recorded) = stub_backend().await;
    let client = MoatClient::new_with_api_key(&base_url, "test-key", HttpConfig::default());
    let config = Config {
        api_url: base_url,
        auth_mode: AuthMode::Standalone { api_key: None },
        session_file: String::new(),
        cookie_name: "session".to_string(),
        login: moatless_cli::config::LoginEnv::default(),
        sandbox_grpc_port: "50052".to_string(),
        gh_repo_set: false,
        github_token_set: false,
        http: HttpConfig::default(),
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let batch = dir.path().join("batch.jsonl");
    std::fs::write(&batch, "\"a\"\n\"fail\"\n\"b\"\n").expect("write the batch");
    let action = DecideAction::Pick(Box::new(PickArgs {
        question: "Which?".to_string(),
        options: vec!["a".to_string(), "b".to_string()],
        option: Vec::new(),
        state: None,
        state_file: Some(batch),
        each: true,
    }));

    let error = run(&client, action, &config, false)
        .await
        .expect_err("one item failed");

    assert_eq!(error.to_string(), "1 of 3 decisions failed");
    assert_eq!(recorded.lock().expect("recorder").len(), 3);
}

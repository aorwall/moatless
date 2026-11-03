use super::*;
use serde_json::json;
use std::time::Duration;

fn params() -> ThreadResumeParams {
    let config = crate::config::test_support::minimal_config();
    let request = AgentRunRequest {
        content: Vec::new(),
        model: Some("gpt-test".to_string()),
        system_prompt: None,
        resume: None,
        agent_profile: None,
        agent_mode: None,
        effort: None,
    };
    thread_resume_params(&config, &request, "gpt-test", "thread-1")
}

fn missing_provider(provider: &str) -> serde_json::Value {
    json!({
        "code": -32600,
        "message": "provider is not configured",
        "data": {"type": "modelProviderNotConfigured", "modelProvider": provider},
    })
}

#[tokio::test]
async fn empty_provider_resume_retries_the_same_thread_with_the_active_provider() {
    let (_events, mut server, session) = AppServerSession::scripted();
    let client = session.resume_thread(params());
    let peer = async {
        let original = server.next_request().await;
        server
            .write(json!({"id": original["id"], "error": missing_provider("")}))
            .await;
        let config = server
            .answer_next(json!({"config": {"model_provider": "gateway"}}))
            .await;
        assert_eq!(config["method"], "config/read");
        assert_eq!(config["params"]["cwd"], original["params"]["cwd"]);
        let retry = server
            .answer_next(json!({"thread": {"id": "thread-1"}}))
            .await;
        assert_eq!(retry["method"], "thread/resume");
        let mut expected = original["params"].clone();
        expected["modelProvider"] = json!("gateway");
        assert_eq!(retry["params"], expected);
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(client, peer) })
            .await
            .expect("resume recovery completed");
    assert_eq!(result.expect("resume recovered"), "thread-1");
}

#[tokio::test]
async fn resume_recovery_stops_after_one_retry() {
    let (_events, mut server, session) = AppServerSession::scripted();
    let client = session.resume_thread(params());
    let peer = async {
        let original = server.next_request().await;
        server
            .write(json!({"id": original["id"], "error": missing_provider("")}))
            .await;
        server
            .answer_next(json!({"config": {"model_provider": "gateway"}}))
            .await;
        let retry = server.next_request().await;
        server
            .write(json!({"id": retry["id"], "error": missing_provider("gateway")}))
            .await;
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(client, peer) })
            .await
            .expect("bounded retry completed");
    assert!(
        matches!(result, Err(RpcError::Rpc(info)) if info.data == Some(json!({"type": "modelProviderNotConfigured", "modelProvider": "gateway"})))
    );
}

#[tokio::test]
async fn an_unresolved_active_provider_preserves_the_original_resume_error() {
    for config in [
        json!({}),
        json!({"model_provider": ""}),
        json!({"model_provider": "  "}),
    ] {
        let (_events, mut server, session) = AppServerSession::scripted();
        let client = session.resume_thread(params());
        let peer = async {
            let original = server.next_request().await;
            server
                .write(json!({"id": original["id"], "error": missing_provider("")}))
                .await;
            server.answer_next(json!({"config": config})).await;
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(client, peer) })
                .await
                .expect("configuration lookup completed");
        assert!(
            matches!(result, Err(RpcError::Rpc(info)) if info.data == Some(json!({"type": "modelProviderNotConfigured", "modelProvider": ""})))
        );
    }
}

#[tokio::test]
async fn named_missing_providers_and_other_resume_errors_are_preserved() {
    for error in [
        missing_provider("retired"),
        json!({"code": -32600, "message": "missing rollout"}),
    ] {
        let (_events, mut server, session) = AppServerSession::scripted();
        let client = session.resume_thread(params());
        let peer = async {
            let request = server.next_request().await;
            server
                .write(json!({"id": request["id"], "error": error}))
                .await;
        };
        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(client, peer) })
                .await
                .expect("resume completed");
        assert!(matches!(result, Err(RpcError::Rpc(info)) if info.message == error["message"]));
    }
}

#[tokio::test]
async fn successful_resume_keeps_the_existing_provider_without_reading_config() {
    let (_events, mut server, session) = AppServerSession::scripted();
    let client = session.resume_thread(params());
    let peer = async {
        let request = server
            .answer_next(json!({"thread": {"id": "thread-1"}}))
            .await;
        assert!(request["params"].get("modelProvider").is_none());
    };
    let (result, ()) = tokio::join!(client, peer);
    assert_eq!(result.expect("resumed"), "thread-1");
}

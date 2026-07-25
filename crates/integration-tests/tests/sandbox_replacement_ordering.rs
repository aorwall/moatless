//! Session continuity across a real Sandbox replacement.
//!
//! These are Agent-tier tests because the contract being proved is observable
//! only after the selected production harness accepts input. Each case opens a
//! real Session, stops the Sandbox, sends a pending Message while it is down,
//! then requires the replacement to acknowledge that Message against the same
//! restored Session.
//!
//! Env gate: `SANDBOX_REPLACEMENT_E2E=1`. The gate deliberately fails when
//! either harness lacks credentials so a configured workflow cannot false-pass.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    enums::MessageRole,
    message::SendMessageRequest,
    shared::{AgentMode, AgentType},
    task::CreateTaskRequest,
};
use moatless_cli::events::EventCollector;

#[derive(Clone, Copy)]
struct HarnessCase {
    agent_type: &'static str,
    model: &'static str,
    model_prefix: &'static str,
    credential_envs: &'static [&'static str],
}

const CLAUDE: HarnessCase = HarnessCase {
    agent_type: "claude-code-cli",
    model: "claude-haiku-4-5",
    model_prefix: "claude-",
    credential_envs: &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
};

const CODEX: HarnessCase = HarnessCase {
    agent_type: "codex",
    model: "gpt-5.4-mini",
    model_prefix: "gpt-",
    credential_envs: &["CODEX_AUTH_JSON", "OPENAI_API_KEY"],
};

#[tokio::test]
#[ignore = "requires Docker Sandbox replacement and Claude credentials"]
async fn replacement_restores_claude_before_accepting_pending_input() {
    if !case_is_enabled(CLAUDE) {
        return;
    }
    assert_replacement_contract(CLAUDE).await;
}

#[tokio::test]
#[ignore = "requires Docker Sandbox replacement and Codex credentials"]
async fn replacement_restores_codex_before_accepting_pending_input() {
    if !case_is_enabled(CODEX) {
        return;
    }
    assert_replacement_contract(CODEX).await;
}

fn case_is_enabled(case: HarnessCase) -> bool {
    if std::env::var("SANDBOX_REPLACEMENT_E2E").as_deref() != Ok("1") {
        return false;
    }
    assert!(
        case.credential_envs
            .iter()
            .any(|key| std::env::var(key).is_ok_and(|value| !value.is_empty())),
        "SANDBOX_REPLACEMENT_E2E requires credentials for {} ({})",
        case.agent_type,
        case.credential_envs.join(" or ")
    );
    true
}

async fn assert_replacement_contract(case: HarnessCase) {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let collector = EventCollector::start(&client)
        .await
        .expect("start event collector");

    let created = client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: Some(repo_id),
            workspace_id: None,
            message: Some("Reply with exactly FIRST and do nothing else.".into()),
            name: Some(format!("{} replacement ordering E2E", case.agent_type)),
            branch: None,
            description: None,
            agent_mode: Some(AgentMode("code".to_string())),
            agent_type: Some(AgentType(case.agent_type.to_string())),
            contexts: vec![],
            file_ids: vec![],
            model: Some(case.model.to_string()),
            headless: None,
            parent_task_id: None,
        })
        .await
        .expect("create task");
    let task_id = &created.task_id;

    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "initial Sandbox did not become ready for {}",
        case.agent_type
    );

    let first_session_event = collector
        .wait_for_nth_event(
            "agent",
            "session_init",
            task_id,
            1,
            Duration::from_secs(180),
        )
        .await
        .expect("selected harness did not open its first Session");
    let first_session = first_session_event
        .payload
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .expect("session_init carries session_id")
        .to_string();

    collector
        .wait_for_nth_event(
            "taskTurn",
            "completed",
            task_id,
            1,
            Duration::from_secs(300),
        )
        .await
        .expect("first Turn did not complete");
    assert_family_model_was_used(&client, task_id, case, None).await;

    let session_events_before = collector.count_events("agent", "session_init", task_id);
    let completed_before = collector.count_events("taskTurn", "completed", task_id);

    client
        .api()
        .stop_sandbox(task_id)
        .await
        .expect("stop Sandbox for replacement");
    assert!(
        sandbox_helpers::wait_for_sandbox_status(
            &client,
            task_id,
            "stopped",
            Duration::from_secs(60),
        )
        .await,
        "Sandbox did not stop before the pending Message was sent"
    );

    // The backend persists the Message before auto-provisioning the replacement.
    // It therefore reaches run_init while dispatch is gated and can be accepted
    // only after the Session archive has been downloaded.
    let pending = client
        .api()
        .send_message(
            task_id,
            &SendMessageRequest {
                message: "Reply with exactly SECOND and do nothing else.".into(),
                resume: Some(true),
                model: Some(case.model.to_string()),
                file_ids: vec![],
                agent_mode: Some(AgentMode("code".to_string())),
                contexts: vec![],
                tool_use_id: None,
                tool_name: None,
                tool_response: None,
                source_task_id: None,
                start_fresh: Some(false),
            },
        )
        .await
        .expect("send pending Message while Sandbox is stopped");
    let pending_uuid = pending
        .uuid
        .expect("regular Message response carries a uuid");

    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "replacement Sandbox did not become ready for {}",
        case.agent_type
    );

    let resumed_session_event = collector
        .wait_for_nth_event(
            "agent",
            "session_init",
            task_id,
            session_events_before + 1,
            Duration::from_secs(180),
        )
        .await
        .expect("replacement harness did not accept the pending Message");
    let resumed_session = resumed_session_event
        .payload
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .expect("replacement session_init carries session_id");
    assert_eq!(
        resumed_session, first_session,
        "{} replacement must resume the restored Session",
        case.agent_type
    );

    collector
        .wait_for_nth_event(
            "taskTurn",
            "completed",
            task_id,
            completed_before + 1,
            Duration::from_secs(300),
        )
        .await
        .expect("replacement Turn did not complete");

    let messages = client
        .api()
        .get_messages(task_id)
        .await
        .expect("read messages after replacement");
    let accepted = messages
        .messages
        .iter()
        .find(|message| message.uuid == pending_uuid)
        .expect("pending Message remains visible after acknowledgement");
    assert_eq!(
        accepted.session_id.as_deref(),
        Some(first_session.as_str()),
        "pending Message must be acknowledged against the restored Session"
    );
    assert_eq!(
        accepted.is_queued,
        Some(false),
        "pending Message must be acknowledged exactly once"
    );
    let replacement_turn = accepted
        .turn_number
        .expect("accepted replacement Message carries a turn number");
    assert_family_model_was_used(&client, task_id, case, Some(replacement_turn)).await;

    let task = client.api().get_task(task_id).await.expect("read task");
    assert_eq!(task.agent_type.as_str(), case.agent_type);

    collector.stop();
    client
        .api()
        .stop_sandbox(task_id)
        .await
        .expect("stop replacement Sandbox during cleanup");
    client
        .api()
        .delete_task(task_id)
        .await
        .expect("delete replacement E2E task");
}

async fn assert_family_model_was_used(
    client: &moatless_cli::client::MoatClient,
    task_id: &str,
    case: HarnessCase,
    turn_number: Option<i32>,
) {
    let messages = client
        .api()
        .get_messages(task_id)
        .await
        .expect("read harness output");
    assert!(
        messages.messages.iter().any(|message| {
            message.role == MessageRole::Assistant
                && turn_number.is_none_or(|turn| message.turn_number == Some(turn))
                && message
                    .model
                    .as_deref()
                    .is_some_and(|model| model.starts_with(case.model_prefix))
        }),
        "{} output must carry a {} model, proving the selected harness ran",
        case.agent_type,
        case.model_prefix
    );
}

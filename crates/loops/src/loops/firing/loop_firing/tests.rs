//! The prompt a firing opens its Task with.

use std::sync::Mutex;

use chrono::Utc;

use moatless_ports::task::{NewTask, TaskOpened};
use moatless_types::access_control::Scope;
use moatless_types::agent::AgentProfile;
use moatless_types::loops::RoutingMode;

use super::*;

/// Captures the `NewTask` the runner opens, which is where the assembled
/// `append_system_prompt` becomes observable.
#[derive(Default)]
struct CapturingOpener(Mutex<Option<NewTask>>);

#[async_trait::async_trait]
impl TaskOpener for CapturingOpener {
    async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError> {
        *self.0.lock().unwrap() = Some(req);
        Ok(TaskOpened {
            task_id: "task-1".to_string(),
            provision: SandboxProvision::Started,
        })
    }
}

fn schedule_context(prompt_template: Option<&str>) -> LoopContext {
    let now = Utc::now();
    LoopContext {
        id: "sch-1".to_string(),
        adapter_kind: SCHEDULED_ORIGIN.to_string(),
        source_id: "sch-1".to_string(),
        source_name: Some("sch-1".to_string()),
        workspace_id: "ws-1".to_string(),
        routing_mode: RoutingMode::BySubject,
        prompt_template: prompt_template.map(str::to_string),
        active: true,
        allow_direct_send: false,
        adapter_config: None,
        ongoing_task_id: None,
        scope: Scope::User,
        task_name: Some("Docs Improver".to_string()),
        tag_ids: vec![],
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: vec![],
        run_as_user_id: Some("user-1".to_string()),
        agent_profile: AgentProfile::ClaudeCode,
        model: None,
        close_after_settled_days: None,
        settle_after_hours: None,
        created_by: "user-1".to_string(),
        created_at: now,
        updated_at: now,
        node_selector: None,
        stop_when_done: false,
        room: None,
    }
}

fn scheduled_message(text: &str) -> InboundMessage {
    InboundMessage {
        adapter_kind: AdapterKind::Webhook,
        source_id: "sch-1".to_string(),
        thread_id: None,
        message_thread_id: "scheduled:sch-1".to_string(),
        user_id: "user-1".to_string(),
        author: None,
        sender_external_id: None,
        requester: None,
        text: text.to_string(),
        file_ids: vec![],
        attachments: Vec::new(),
        thread_properties: serde_json::Value::Null,
        adapter_context: serde_json::Value::Null,
        external_event_id: Some("scheduled:sch-1:1780000000".to_string()),
        source_task_id: None,
        audience: Audience::Everyone,
        replies_to: Vec::new(),
        event: None,
    }
}

async fn opened_prompt(config: &LoopContext) -> Option<String> {
    let opener = CapturingOpener::default();
    open_task(&opener, None, config, &scheduled_message("/docs-improver"))
        .await
        .expect("the opener accepts every task");
    let captured = opener.0.lock().unwrap().take();
    captured
        .expect("the runner opened a task")
        .append_system_prompt
}

/// The agent otherwise reads the same prompt an interactive one does. A
/// scheduled `docs-improver` run stopped at its assignment step and asked
/// the user to pick an option, which nobody was there to answer.
#[tokio::test]
async fn a_scheduled_task_is_told_that_nobody_is_waiting_on_it() {
    let config = schedule_context(Some("Run the `docs-improver` skill."));

    let prompt = opened_prompt(&config)
        .await
        .expect("a scheduled firing always has a prompt");

    assert!(prompt.starts_with(SCHEDULE_ORIGIN_CONTEXT));
    assert!(prompt.ends_with("Run the `docs-improver` skill."));
}

/// A Loop with no prompt of its own still gets the origin context.
#[tokio::test]
async fn the_origin_context_does_not_depend_on_a_loop_prompt() {
    let config = schedule_context(None);

    assert_eq!(
        opened_prompt(&config).await.as_deref(),
        Some(SCHEDULE_ORIGIN_CONTEXT)
    );
}

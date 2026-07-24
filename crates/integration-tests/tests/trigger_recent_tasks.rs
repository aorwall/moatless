//! Trigger-detail "recent tasks" listing.
//!
//! Verifies that the `GET /api/v1/triggers/{id}/tasks` endpoint is driven by
//! `tasks.trigger_id` (not `task_bindings.trigger_id`), so schedule triggers —
//! which never create a binding — still surface their tasks. Adapter-event
//! triggers continue to return a populated binding.

use integration_tests::{
    create_webhook_integration, ensure_template_repo, exec_psql, raw_api, test_client,
};
use moatless_api_schemas::enums::RoutingMode;
use moatless_api_schemas::loops::{
    CreateSchedule, CreateLoopRequest, CreateLoopConfig, CreateLoopSource,
    LoopKind,
};
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

/// Insert a task row directly with `trigger_id` set, simulating what the
/// schedule runner produces on tick. The DB-tier test cannot exercise the
/// 30-second cron loop, so we stamp the row directly.
fn insert_scheduled_task_row(task_id: &str, repository_id: &str, trigger_id: &str) {
    let sql = format!(
        r#"
        INSERT INTO tasks (
            task_id, short_id, created_at, last_message_at, agent_type,
            repository_id, status, total_cost_usd, visibility, scope,
            team_id, headless, ttl_minutes, inactive_ttl_days, trigger_id
        ) VALUES (
            '{task_id}',
            '{short}',
            (now() AT TIME ZONE 'UTC')::text,
            (now() AT TIME ZONE 'UTC')::text,
            'claude-code',
            '{repository_id}',
            'open',
            0,
            'private',
            'global',
            'system',
            true,
            15,
            7,
            '{trigger_id}'
        )
        "#,
        short = &task_id[..8],
    );
    exec_psql(&sql);
}

#[tokio::test]
async fn schedule_trigger_recent_tasks_returns_binding_none() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    // Create a schedule trigger. Cron expression doesn't matter — we won't wait
    // for it; we stamp the task row directly below.
    let trigger = client
        .api()
        .create_trigger(&CreateLoopRequest {
            name: Some(format!("recent-tasks-schedule-{suffix}")),
            kind: LoopKind::Schedule,
            active: Some(true),
            scope: None,
            team_id: None,
            rule: CreateLoopConfig {
                repository_id: repo_id.clone(),
                routing_mode: RoutingMode::BySubject,
                ongoing_task_id: None,
                task_name: Some("Scheduled recent-tasks check".into()),
                prompt: None,
                tag_ids: None,
                run_as_user_id: None,
                agent_type: None,
                inactive_ttl_days: None,
            },
            source: CreateLoopSource::Schedule(CreateSchedule {
                cron_expression: "0 0 * * * * *".into(),
                timezone: Some("UTC".into()),
                message_template: format!("scheduled-tick-{suffix}"),
            }),
        })
        .await
        .unwrap();

    let task_id = uuid::Uuid::new_v4().to_string();
    insert_scheduled_task_row(&task_id, &repo_id, &trigger.id);

    let items = client
        .api()
        .list_trigger_recent_tasks(&trigger.id, Some(10))
        .await
        .unwrap();

    assert_eq!(items.len(), 1, "expected exactly one recent task");
    let item = &items[0];
    assert_eq!(item.task.task_id, task_id);
    assert!(
        item.binding.is_none(),
        "schedule triggers must not return a binding, got: {:?}",
        item.binding
    );

    // Cleanup
    exec_psql(&format!("DELETE FROM tasks WHERE task_id = '{task_id}'"));
    let _ = client.api().delete_trigger(&trigger.id).await;
}

#[tokio::test]
async fn adapter_event_trigger_recent_tasks_populates_binding() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("recent-tasks-adapter-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;

    let payload = serde_json::json!({ "text": "trigger-recent-binding-test" });
    let resp = raw_api::send_webhook_message(&http, &fixture.trigger.id, &payload).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let task_id = body["taskId"].as_str().expect("taskId in webhook response");

    let items = client
        .api()
        .list_trigger_recent_tasks(&fixture.trigger.id, Some(10))
        .await
        .unwrap();

    let item = items
        .iter()
        .find(|it| it.task.task_id == task_id)
        .expect("created task appears in trigger recent-tasks");
    let binding = item
        .binding
        .as_ref()
        .expect("adapter-event trigger should yield a populated binding");
    assert!(!binding.id.is_empty());
    assert!(!binding.subject_key.is_empty());

    let _ = client.api().stop_task(task_id).await;
    let _ = client.api().delete_task(task_id).await;
}

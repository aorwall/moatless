//! Canonical integration route/trigger CRUD smoke tests.

use chrono::Utc;
use integration_tests::{create_webhook_integration, ensure_template_repo, test_client};
use moatless_api_schemas::enums::RoutingMode;
use moatless_api_schemas::triggers::{
    CreateScheduleTriggerSource, CreateTriggerRequest, CreateTriggerRule, CreateTriggerSource,
    TriggerKind, TriggerSource, UpdateScheduleTriggerSource, UpdateTriggerRequest,
    UpdateTriggerRule, UpdateTriggerSource,
};
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

#[tokio::test]
async fn task_route_trigger_crud_chain() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let source = format!("crud-test-{suffix}");

    let fixture =
        create_webhook_integration(&client, &repo_id, &source, RoutingMode::BySubject, None).await;

    assert!(!fixture.connection.id.is_empty());
    assert_eq!(fixture.connection.external_account_id, source);
    assert!(!fixture.trigger.id.is_empty());
    assert_eq!(fixture.trigger.rule.repository_id, repo_id);
    assert_eq!(fixture.trigger.rule.routing_mode, RoutingMode::BySubject);
    assert!(fixture.trigger.active);
    let TriggerSource::AdapterEvent(source_config) = &fixture.trigger.source else {
        panic!("expected adapter-event trigger source");
    };
    assert_eq!(source_config.source_matcher, source);

    let triggers = client
        .api()
        .list_triggers(Some("adapter_event"), Some("webhook"))
        .await
        .unwrap();
    assert!(triggers
        .iter()
        .any(|trigger| trigger.id == fixture.trigger.id));

    let updated_trigger = client
        .api()
        .update_trigger(
            &fixture.trigger.id,
            &UpdateTriggerRequest {
                rule: Some(UpdateTriggerRule {
                    prompt: Some(Some("Please help: {message}".into())),
                    task_name: Some(Some("Updated webhook task".into())),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated_trigger.rule.prompt.as_deref(),
        Some("Please help: {message}")
    );
    assert_eq!(
        updated_trigger.rule.task_name.as_deref(),
        Some("Updated webhook task")
    );

    let deactivated = client
        .api()
        .update_trigger(
            &fixture.trigger.id,
            &UpdateTriggerRequest {
                active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!deactivated.active);

    let triggers = client
        .api()
        .list_triggers(Some("adapter_event"), Some("webhook"))
        .await
        .unwrap();
    assert!(triggers
        .iter()
        .any(|trigger| trigger.id == fixture.trigger.id && !trigger.active));
}

#[tokio::test]
async fn schedule_trigger_crud_and_next_fire_chain() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let before_create = Utc::now();

    let created = client
        .api()
        .create_trigger(&CreateTriggerRequest {
            name: Some(format!("scheduled-crud-{suffix}")),
            kind: TriggerKind::Schedule,
            active: Some(true),
            scope: None,
            team_id: None,
            rule: CreateTriggerRule {
                repository_id: repo_id.clone(),
                routing_mode: RoutingMode::BySubject,
                ongoing_task_id: None,
                task_name: Some("Scheduled smoke task".into()),
                prompt: Some("Scheduled prompt: {message}".into()),
                tag_ids: None,
                run_as_user_id: None,
                agent_type: Some("codex".into()),
                inactive_ttl_days: None,
            },
            source: CreateTriggerSource::Schedule(CreateScheduleTriggerSource {
                cron_expression: "0/5 * * * * * *".into(),
                timezone: Some("UTC".into()),
                message_template: "Scheduled smoke message".into(),
            }),
        })
        .await
        .unwrap();

    assert_eq!(created.kind, TriggerKind::Schedule);
    assert_eq!(created.rule.repository_id, repo_id);
    assert_eq!(created.rule.routing_mode, RoutingMode::BySubject);
    assert_eq!(
        created.rule.task_name.as_deref(),
        Some("Scheduled smoke task")
    );
    assert_eq!(
        created.rule.prompt.as_deref(),
        Some("Scheduled prompt: {message}")
    );

    let TriggerSource::Schedule(created_source) = &created.source else {
        panic!("expected schedule trigger source");
    };
    assert_eq!(created_source.cron_expression, "0/5 * * * * * *");
    assert_eq!(created_source.timezone, "UTC");
    assert_eq!(created_source.message_template, "Scheduled smoke message");
    assert!(created_source.last_fired_at.is_none());

    let next_fire = chrono::DateTime::parse_from_rfc3339(&created_source.next_fire_at)
        .unwrap()
        .with_timezone(&Utc);
    assert!(next_fire > before_create);

    let schedules = client
        .api()
        .list_triggers(Some("schedule"), None)
        .await
        .unwrap();
    assert!(schedules.iter().any(|trigger| trigger.id == created.id));

    let before_update = Utc::now();
    let updated = client
        .api()
        .update_trigger(
            &created.id,
            &UpdateTriggerRequest {
                source: Some(UpdateTriggerSource::Schedule(UpdateScheduleTriggerSource {
                    cron_expression: Some("0/10 * * * * * *".into()),
                    timezone: Some("Europe/Stockholm".into()),
                    message_template: Some("Updated scheduled smoke message".into()),
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let TriggerSource::Schedule(updated_source) = &updated.source else {
        panic!("expected schedule trigger source");
    };
    assert_eq!(updated_source.cron_expression, "0/10 * * * * * *");
    assert_eq!(updated_source.timezone, "Europe/Stockholm");
    assert_eq!(
        updated_source.message_template,
        "Updated scheduled smoke message"
    );

    let updated_next_fire = chrono::DateTime::parse_from_rfc3339(&updated_source.next_fire_at)
        .unwrap()
        .with_timezone(&Utc);
    assert!(updated_next_fire > before_update);
}

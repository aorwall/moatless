//! Canonical integration route/trigger CRUD smoke tests.

use integration_tests::{create_webhook_integration, ensure_template_repo, test_client};
use moatless_api_schemas::channel::{UpdateTaskRouteRequest, UpdateTaskTriggerRequest};
use moatless_api_schemas::enums::RoutingMode;
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
    assert!(!fixture.route.id.is_empty());
    assert_eq!(fixture.route.repository_id, repo_id);
    assert_eq!(fixture.route.routing_mode, RoutingMode::BySubject);
    assert!(!fixture.trigger.id.is_empty());
    assert_eq!(fixture.trigger.route_id, fixture.route.id);
    assert_eq!(fixture.trigger.source_matcher, source);
    assert!(fixture.trigger.active);

    let routes = client.api().list_task_routes(Some(&repo_id)).await.unwrap();
    assert!(routes.iter().any(|route| route.id == fixture.route.id));

    let triggers = client
        .api()
        .list_task_triggers(Some("webhook"), Some(&fixture.route.id))
        .await
        .unwrap();
    assert!(triggers
        .iter()
        .any(|trigger| trigger.id == fixture.trigger.id));

    let updated_route = client
        .api()
        .update_task_route(
            &fixture.route.id,
            &UpdateTaskRouteRequest {
                prompt_template: Some(Some("Please help: {message}".into())),
                task_name: Some(Some("Updated webhook task".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated_route.prompt_template.as_deref(),
        Some("Please help: {message}")
    );
    assert_eq!(
        updated_route.task_name.as_deref(),
        Some("Updated webhook task")
    );

    let deactivated = client
        .api()
        .update_task_trigger(
            &fixture.trigger.id,
            &UpdateTaskTriggerRequest {
                active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!deactivated.active);

    let triggers = client
        .api()
        .list_task_triggers(Some("webhook"), Some(&fixture.route.id))
        .await
        .unwrap();
    assert!(triggers
        .iter()
        .any(|trigger| trigger.id == fixture.trigger.id && !trigger.active));
}

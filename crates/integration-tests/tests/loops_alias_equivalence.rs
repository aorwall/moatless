//! `/api/v1/loops` and its deprecated `/api/v1/triggers` alias must serve the
//! same handlers, so the same operation returns equivalent results on either
//! path. Ticket 03 exposes `/api/v1/loops` and keeps `/api/v1/triggers` working
//! (undocumented) for existing clients.

use integration_tests::{create_webhook_integration, ensure_template_repo, test_client};
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
async fn loops_and_triggers_alias_return_equivalent_results() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let source = format!("loop-alias-{suffix}");

    let fixture =
        create_webhook_integration(&client, &repo_id, &source, RoutingMode::BySubject, None).await;
    let loop_id = fixture.trigger.id.clone();

    // GET by id: /loops/{id} and /triggers/{id} return the same Loop.
    let via_loops = client.api().get_loop(&loop_id).await.unwrap();
    let via_triggers = client.api().get_trigger(&loop_id).await.unwrap();
    assert_eq!(
        serde_json::to_value(&via_loops).unwrap(),
        serde_json::to_value(&via_triggers).unwrap(),
        "GET /loops/{{id}} and GET /triggers/{{id}} must be identical"
    );

    // LIST: the created Loop appears on both paths, and the full lists match.
    let loops_list = client
        .api()
        .list_loops(Some("adapter_event"), Some("webhook"))
        .await
        .unwrap();
    let triggers_list = client
        .api()
        .list_triggers(Some("adapter_event"), Some("webhook"))
        .await
        .unwrap();
    assert!(
        loops_list.iter().any(|l| l.id == loop_id),
        "created Loop must appear on /api/v1/loops"
    );
    assert!(
        triggers_list.iter().any(|l| l.id == loop_id),
        "created Loop must appear on /api/v1/triggers"
    );
    assert_eq!(
        serde_json::to_value(&loops_list).unwrap(),
        serde_json::to_value(&triggers_list).unwrap(),
        "LIST /loops and /triggers must be identical"
    );

    // Cleanup via the documented path.
    let _ = client.api().delete_loop(&loop_id).await;
}

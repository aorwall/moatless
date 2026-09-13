use super::conformance::tab;
use super::redis::decode_tabs;
use super::{InMemoryPreviewTabDao, PreviewTabDao, conformance};
use serde_json::json;
use std::collections::HashMap;

/// Each case gets its own handle, so one failing does not leave state for the
/// next and a failure names the behaviour that broke.
#[tokio::test]
async fn opening_a_tab_lists_it() {
    conformance::opening_a_tab_lists_it(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn revisions_only_move_forward() {
    conformance::revisions_only_move_forward(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn navigating_replaces_the_whole_tab() {
    conformance::navigating_replaces_the_whole_tab(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn closing_without_a_tab_id_closes_every_tab() {
    conformance::closing_without_a_tab_id_closes_every_tab(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn closing_an_unknown_tab_removes_nothing() {
    conformance::closing_an_unknown_tab_removes_nothing(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn a_published_event_reaches_a_subscriber() {
    conformance::a_published_event_reaches_a_subscriber(&InMemoryPreviewTabDao::new()).await;
}

#[tokio::test]
async fn the_epoch_is_stable_for_the_life_of_a_handle() {
    let store = InMemoryPreviewTabDao::new();
    assert_eq!(store.epoch(), store.epoch());
    assert!(!store.epoch().is_empty());
    assert_ne!(store.epoch(), InMemoryPreviewTabDao::new().epoch());
}

#[test]
fn a_stored_tab_round_trips_through_the_wire_shape() {
    let encoded = serde_json::to_value(tab("task", "a", "https://a")).unwrap();
    assert_eq!(
        encoded,
        json!({
            "threadId": "task",
            "tabId": "a",
            "navStatus": { "_tag": "Success", "url": "https://a", "title": "" },
            "canGoBack": false,
            "canGoForward": false,
            "updatedAt": "2026-07-31T00:00:00Z"
        })
    );
}

#[test]
fn an_undecodable_field_is_skipped_rather_than_failing_the_listing() {
    let fields = HashMap::from([
        (
            "a".to_string(),
            serde_json::to_string(&tab("task", "a", "https://a")).unwrap(),
        ),
        ("b".to_string(), "not json".to_string()),
    ]);
    assert_eq!(decode_tabs(fields), vec![tab("task", "a", "https://a")]);
}

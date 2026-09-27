#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on"
)]
//! Which events the installed Adapters actually subscribe to.
//!
//! The event-handler consumer names no provider: it hands every event to
//! [`TaskEventSubscribers`], so an Adapter's registration is the contract — a
//! silent unsubscribe is an outage no other test here would notice.
//!
//! Dispatch semantics (overlapping subscribers, fault isolation) are pinned in
//! `moatless-loops`'s own unit tests, which need no `AppState`.

use crate::common;

use sqlx::PgPool;

use moatless_tasks::agent_event::EventKey;
use moatless_types::agent::AgentSubType;
use moatless_types::message::MessageSubType;
use moatless_types::sandbox::SandboxStatus;

use common::{test_app_state_with_adapters, test_config};

/// Every event Linear's live feed and the GitHub shell claim read has a
/// subscriber, checked key by key so a single dropped key isn't hidden by a
/// count.
#[sqlx::test]
async fn the_installed_adapters_subscribe_to_the_events_they_react_to(pool: PgPool) {
    let state = test_app_state_with_adapters(pool, test_config());
    let installations = state
        .adapter_installations
        .clone()
        .expect("adapters are installed");

    let keys = installations.event_subscribers(&state).keys();

    for expected in [
        // Linear's live session feed.
        EventKey::Agent(AgentSubType::Started),
        EventKey::Agent(AgentSubType::Completed),
        EventKey::Agent(AgentSubType::Stopped),
        EventKey::Agent(AgentSubType::Error),
        EventKey::Sandbox(SandboxStatus::Error),
        EventKey::Sandbox(SandboxStatus::Initializing),
        EventKey::Sandbox(SandboxStatus::Ready),
        EventKey::Sandbox(SandboxStatus::Stopped),
        // Linear mirrors the transcript from the Projector's output.
        EventKey::Message(MessageSubType::Upsert),
    ] {
        assert!(
            keys.contains(&expected),
            "no Adapter subscribed to {expected:?}; registered: {keys:?}"
        );
    }
}

/// An event no Adapter asked for stays unsubscribed.
///
/// The registry is keyed, not a broadcast.
#[sqlx::test]
async fn an_event_no_adapter_asked_for_has_no_subscriber(pool: PgPool) {
    let state = test_app_state_with_adapters(pool, test_config());
    let installations = state
        .adapter_installations
        .clone()
        .expect("adapters are installed");

    let keys = installations.event_subscribers(&state).keys();

    assert!(
        !keys.contains(&EventKey::Agent(AgentSubType::SessionInit)),
        "agent.session_init is generic handling only: {keys:?}"
    );
}

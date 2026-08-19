#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on"
)]
//! Which events the installed Adapters actually subscribe to.
//!
//! The event-handler consumer no longer names a provider: it hands every event
//! to [`TaskEventSubscribers`], and what an Adapter reacts to is decided by its
//! own registration. That makes the registration the contract, and a provider
//! that silently stops subscribing is a live-progress outage no other test in
//! this repository would notice — `dispatch_event` has never had one.
//!
//! The dispatch semantics themselves (overlapping subscribers, non-matching
//! keys, fault isolation) are pinned where the seam lives, in
//! `moatless-loops`'s own unit tests; they need no database and no `AppState`.

use crate::common;

use sqlx::PgPool;

use moatless_types::agent::AgentSubType;
use moatless_types::sandbox::SandboxStatus;
use moatless_tasks::agent_event::EventKey;

use common::{test_app_state_with_adapters, test_config};

/// Every event a Linear session's live feed is built from has a subscriber, and
/// so does the one the GitHub shell claim reads.
///
/// Listed key by key rather than counted: a dropped key is exactly the
/// regression this exists to catch, and a count hides it behind any other
/// addition.
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
        // Both Adapters read this one — Linear for the transcript, GitHub PR
        // for the shell claim.
        EventKey::Agent(AgentSubType::Message),
    ] {
        assert!(
            keys.contains(&expected),
            "no Adapter subscribed to {expected:?}; registered: {keys:?}"
        );
    }
}

/// An event no Adapter asked for stays unsubscribed.
///
/// The registry is keyed, not a broadcast: a subscriber that reacted to every
/// event would make the keys above meaningless.
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

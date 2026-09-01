//! Admission and relay behaviour (the durable-ingress decision), driven through the seams the
//! gRPC ingress and the relay actually use.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::task_event::dao::MockTaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::{SettledTurn, TurnSignals, TurnTransition, UnclosedTurn};
use moatless_core::events::EventPublisher;
use moatless_core::events::test_support::RecordingPublisher;

use super::outbox::{ClaimedPayload, IngressOutbox, IngressPayload, Pending};
use super::relay::IngressRelay;
use super::{AdmissionError, AdmissionRequest, EventAdmission};

/// Shared ordered log of side effects, so we can assert commit-before-fan-out.
type Recorder = Arc<Mutex<Vec<&'static str>>>;

// =========================================================================
// Doubles
// =========================================================================

/// Turn repo double that records a `"commit"` on `apply_ingress_event`, then
/// optionally fails.
struct RecordingTurns {
    rec: Recorder,
    fail: bool,
}

#[async_trait]
impl TaskTurnDao for RecordingTurns {
    async fn apply_ingress_event(
        &self,
        _ev: &crate::task_turn::dao::TurnStateEvent,
    ) -> Result<(), sqlx::Error> {
        self.rec.lock().expect("recorder poisoned").push("commit");
        if self.fail {
            return Err(sqlx::Error::PoolClosed);
        }
        Ok(())
    }

    async fn fork_point(
        &self,
        _task_id: &str,
        _at_turn: Option<i32>,
    ) -> Result<crate::task_turn::dao::ForkPoint, sqlx::Error> {
        unreachable!("fork_point is the fork handler's, not the ingress path")
    }

    async fn project_turn(
        &self,
        _task_id: &str,
        _turn_number: i32,
        _agent_type: Option<String>,
        _transition: TurnTransition,
        _at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        unreachable!("project_turn is the consumer fallback, not the ingress path")
    }

    async fn list_unclosed_active_turns(
        &self,
        _older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        unreachable!("list_unclosed_active_turns is the reaper path")
    }

    async fn list_settled_turns_with_later_activity(
        &self,
    ) -> Result<Vec<SettledTurn>, sqlx::Error> {
        unreachable!("list_settled_turns_with_later_activity is the reaper path")
    }

    async fn list_superseded_open_turns(
        &self,
        _older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        unreachable!("list_superseded_open_turns is the reaper path")
    }

    async fn resolve_turn_for_run(
        &self,
        _task_id: &str,
        _run_id: &str,
    ) -> Result<i32, sqlx::Error> {
        Ok(1)
    }

    async fn latest_running_signals(&self, _task_id: &str) -> Result<TurnSignals, sqlx::Error> {
        unreachable!("latest_running_signals is the live-status path, not ingress")
    }

    async fn latest_turn_number(&self, _task_id: &str) -> Result<Option<i32>, sqlx::Error> {
        Ok(Some(1))
    }

    async fn latest_closed_turn_status(
        &self,
        _task_id: &str,
    ) -> Result<Option<crate::task_turn::models::TurnStatus>, sqlx::Error> {
        unreachable!("latest_closed_turn_status is the pending-message path, not ingress")
    }

    async fn set_turn_metrics(
        &self,
        _task_id: &str,
        _turn_number: i32,
        _metrics: &serde_json::Value,
    ) -> Result<bool, sqlx::Error> {
        unreachable!("set_turn_metrics is the completion path, not the ingress path")
    }

    async fn set_agent_turn_id(
        &self,
        _task_id: &str,
        _turn_number: i32,
        _agent_turn_id: &str,
    ) -> Result<bool, sqlx::Error> {
        unreachable!("set_agent_turn_id is the session-init path, not the ingress path")
    }

    async fn list_turn_metrics(
        &self,
        _task_id: &str,
    ) -> Result<Vec<serde_json::Value>, sqlx::Error> {
        unreachable!("list_turn_metrics is the statistics path, not the ingress path")
    }

    async fn increment_recovery_attempts(
        &self,
        _task_id: &str,
        _turn_number: i32,
    ) -> Result<i32, sqlx::Error> {
        unreachable!("increment_recovery_attempts is the recovery path")
    }

    async fn reset_recovery_attempts(
        &self,
        _task_id: &str,
        _turn_number: i32,
    ) -> Result<(), sqlx::Error> {
        unreachable!("reset_recovery_attempts is the recovery path")
    }
}

/// In-memory ingress outbox: admits, claims and deletes like the Postgres one,
/// so admission and relay behaviour can be asserted without a database.
#[derive(Default)]
struct FakeOutbox {
    rec: Option<Recorder>,
    records: Mutex<Vec<(IngressPayload, DateTime<Utc>, i32)>>,
    fail_admit: bool,
}

impl FakeOutbox {
    fn with_log(rec: &Recorder) -> Self {
        Self {
            rec: Some(rec.clone()),
            ..Self::default()
        }
    }

    fn failing() -> Self {
        Self {
            fail_admit: true,
            ..Self::default()
        }
    }

    fn event_ids(&self) -> Vec<String> {
        self.records
            .lock()
            .expect("outbox poisoned")
            .iter()
            .map(|(payload, ..)| payload.event_id.clone())
            .collect()
    }
}

#[async_trait]
impl IngressOutbox for FakeOutbox {
    async fn admit(
        &self,
        payload: &IngressPayload,
        first_attempt_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        if let Some(rec) = &self.rec {
            rec.lock().expect("recorder poisoned").push("spool");
        }
        if self.fail_admit {
            return Err(sqlx::Error::PoolClosed);
        }
        let mut records = self.records.lock().expect("outbox poisoned");
        if !records
            .iter()
            .any(|(existing, ..)| existing.event_id == payload.event_id)
        {
            records.push((payload.clone(), first_attempt_at, 0));
        }
        Ok(())
    }

    async fn delete_delivered(&self, event_id: &str) -> Result<bool, sqlx::Error> {
        let mut records = self.records.lock().expect("outbox poisoned");
        let before = records.len();
        records.retain(|(payload, ..)| payload.event_id != event_id);
        Ok(records.len() < before)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ClaimedPayload>, sqlx::Error> {
        let mut records = self.records.lock().expect("outbox poisoned");
        let mut claimed = Vec::new();
        for (payload, next_attempt_at, attempts) in records.iter_mut() {
            if claimed.len() as i64 >= limit || *next_attempt_at > now {
                continue;
            }
            claimed.push(ClaimedPayload {
                payload: payload.clone(),
                attempts: *attempts,
                admitted_at: now,
            });
            *attempts += 1;
            *next_attempt_at = now + Duration::seconds(30);
        }
        Ok(claimed)
    }

    async fn pending_counts(&self, _stale_before: DateTime<Utc>) -> Result<Pending, sqlx::Error> {
        Ok(Pending {
            total: self.records.lock().expect("outbox poisoned").len() as i64,
            stale: 0,
        })
    }
}

// =========================================================================
// Fixtures
// =========================================================================

fn message_request(event_id: &str) -> AdmissionRequest {
    AdmissionRequest {
        event_id: event_id.to_string(),
        task_id: "task-1".to_string(),
        event_type: "agent",
        sub_type: "message".to_string(),
        turn_number: Some(1),
        payload: serde_json::json!({
            "uuid": "msg-1",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] }
        }),
        state: None,
    }
}

fn state_request(event_id: &str) -> AdmissionRequest {
    AdmissionRequest {
        event_id: event_id.to_string(),
        task_id: "task-1".to_string(),
        event_type: "taskTurn",
        sub_type: "active".to_string(),
        turn_number: Some(1),
        payload: serde_json::json!({}),
        state: Some(crate::task_turn::dao::TurnStateEvent {
            event_id: event_id.to_string(),
            task_id: "task-1".to_string(),
            event_type: "taskTurn".to_string(),
            sub_type: "active".to_string(),
            is_error: false,
            agent_type: Some("claude-code".to_string()),
            turn_number: 1,
            payload: serde_json::json!({}),
            timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            transition: TurnTransition::Open,
        }),
    }
}

fn plain_request(event_id: &str) -> AdmissionRequest {
    AdmissionRequest {
        event_id: event_id.to_string(),
        task_id: "task-1".to_string(),
        event_type: "sandbox",
        sub_type: "ready".to_string(),
        turn_number: None,
        payload: serde_json::json!({ "message": "ready" }),
        state: None,
    }
}

/// Event-row writer double recording an `"event-row"` on `insert_event`.
fn recording_events(rec: &Recorder, fail: bool) -> Arc<dyn crate::task_event::dao::TaskEventDao> {
    let mut events = MockTaskEventDao::new();
    let rec = rec.clone();
    events.expect_insert_event().returning(move |_| {
        rec.lock().expect("recorder poisoned").push("event-row");
        if fail {
            return Err(sqlx::Error::PoolClosed);
        }
        Ok(())
    });
    Arc::new(events)
}

// =========================================================================
// Admission
// =========================================================================

#[tokio::test]
async fn state_is_committed_before_fan_out() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let turns: Arc<dyn TaskTurnDao> = Arc::new(RecordingTurns {
        rec: rec.clone(),
        fail: false,
    });
    let publisher: Arc<dyn EventPublisher> =
        Arc::new(RecordingPublisher::with_order_log(rec.clone()));

    EventAdmission {
        task_turns: Some(&turns),
        task_events: None,
        ingress_outbox: None,
        publisher: Some(&publisher),
    }
    .admit(state_request("evt-1"))
    .await
    .expect("state-bearing event is admitted");

    // The state commit is observed strictly before the fan-out, never the
    // other way round.
    assert_eq!(
        *rec.lock().expect("recorder poisoned"),
        ["commit", "publish"]
    );
}

#[tokio::test]
async fn failed_state_commit_is_retryable_and_never_fans_out() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let turns: Arc<dyn TaskTurnDao> = Arc::new(RecordingTurns {
        rec: rec.clone(),
        fail: true,
    });
    let publisher: Arc<dyn EventPublisher> =
        Arc::new(RecordingPublisher::with_order_log(rec.clone()));

    let error = EventAdmission {
        task_turns: Some(&turns),
        task_events: None,
        ingress_outbox: None,
        publisher: Some(&publisher),
    }
    .admit(state_request("evt-2"))
    .await
    .expect_err("a failed state commit is not an admission");

    assert!(error.is_retryable());
    // No publish: an event whose state never committed must not be fanned out
    // as though it had.
    assert_eq!(*rec.lock().expect("recorder poisoned"), ["commit"]);
    // The cause survives for the log; only the transport response is redacted.
    assert!(
        std::error::Error::source(&error).is_some(),
        "commit failures keep their source"
    );
}

#[tokio::test]
async fn plain_event_commits_its_row_before_fan_out() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, false);
    let publisher: Arc<dyn EventPublisher> =
        Arc::new(RecordingPublisher::with_order_log(rec.clone()));

    EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: None,
        publisher: Some(&publisher),
    }
    .admit(plain_request("evt-3"))
    .await
    .expect("a committed row is a durable admission");

    assert_eq!(
        *rec.lock().expect("recorder poisoned"),
        ["event-row", "publish"]
    );
}

#[tokio::test]
async fn failed_row_commit_is_retryable() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, true);
    let publisher: Arc<dyn EventPublisher> =
        Arc::new(RecordingPublisher::with_order_log(rec.clone()));

    let error = EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: None,
        publisher: Some(&publisher),
    }
    .admit(plain_request("evt-4"))
    .await
    .expect_err("a failed row commit is not an admission");

    assert!(error.is_retryable());
    assert_eq!(*rec.lock().expect("recorder poisoned"), ["event-row"]);
}

#[tokio::test]
async fn agent_message_is_spooled_before_fan_out_and_published_whole() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let outbox = Arc::new(FakeOutbox::with_log(&rec));
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let recorder = Arc::new(RecordingPublisher::with_order_log(rec.clone()));
    let publisher: Arc<dyn EventPublisher> = recorder.clone();
    let events = recording_events(&rec, false);

    EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: Some(&outbox_dyn),
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-5"))
    .await
    .expect("a spooled payload is a durable admission");

    // The outbox record is the whole of what gRPC success speaks for.
    assert_eq!(
        *rec.lock().expect("recorder poisoned"),
        ["spool", "publish"]
    );
    assert_eq!(outbox.event_ids(), ["evt-5"]);

    // The consumer rebuilds the conversation from the published payload, so it
    // must never be eligible for truncation — and it must carry full content,
    // not the relay skeleton.
    let published = recorder.raw_publishes();
    assert_eq!(published.len(), 1);
    assert!(!published[0].payload_recoverable);
    assert_eq!(
        published[0].payload["message"]["content"][0]["text"],
        "hello"
    );
}

#[tokio::test]
async fn agent_message_is_admitted_when_the_transport_fails() {
    // Redis down: the payload is durable in the outbox, so the sandbox is told
    // the event is admitted and must not resend it. The relay owns delivery.
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher::failing(1));
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, false);

    EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: Some(&outbox_dyn),
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-6"))
    .await
    .expect("durably admitted despite the transport failure");

    assert_eq!(
        outbox.event_ids(),
        ["evt-6"],
        "the record must survive for the relay"
    );
}

#[tokio::test]
async fn agent_message_without_a_transport_is_not_admitted() {
    // Redis disabled entirely: full message content has no delivery path and no
    // durable substitute, so the backend must not acknowledge it — and must not
    // spool a payload nothing can ever deliver.
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();

    let error = EventAdmission {
        task_turns: None,
        task_events: None,
        ingress_outbox: Some(&outbox_dyn),
        publisher: None,
    }
    .admit(message_request("evt-7"))
    .await
    .expect_err("no delivery path means no admission");

    assert!(error.is_retryable());
    assert!(outbox.event_ids().is_empty());
}

/// The delivery-path predicate is the transport, deliberately not the consumer.
///
/// Reviewers keep reading `EVENT_HANDLER_ENABLED=false` as "nothing will ever
/// consume this" and proposing that admission refuse in that case. It means
/// "not in *this* process": the consumer group is the shared constant
/// `event-handler`, and a shadow backend sets the flag precisely because it
/// shares the primary's Redis and Postgres, so the primary consumes and deletes
/// the record on its behalf (`scripts/shadow/sync-env.sh`).
///
/// Folding consumer presence into the predicate would therefore refuse
/// admission in the one configuration the flag exists to support, and the
/// sandbox would retry forever against a delivery path that works. If this test
/// fails, that is the change being made — a genuinely consumer-less deployment
/// surfaces at runtime through `event_ingress_outbox_stalled`, not here.
#[tokio::test]
async fn agent_message_is_admitted_when_no_consumer_runs_in_this_process() {
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher::new());
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, false);

    // Nothing here consumes: no event handler, no `task_messages` writer. Only
    // the transport this process owns is present.
    EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: Some(&outbox_dyn),
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-shadow"))
    .await
    .expect("a transport is the whole delivery obligation this process has");

    assert_eq!(outbox.event_ids(), ["evt-shadow"]);
}

#[tokio::test]
async fn agent_message_without_an_outbox_is_not_admitted() {
    let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher::new());
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, false);

    let error = EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: None,
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-8"))
    .await
    .expect_err("no durable spool means no admission");

    assert!(error.is_retryable());
}

/// An `agent.message` never touches `task_events`, so a failing event repository
/// has no say in whether one is admitted.
#[tokio::test]
async fn an_agent_message_is_admitted_with_no_event_row_at_all() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, true);
    let outbox = Arc::new(FakeOutbox::with_log(&rec));
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let recorder = Arc::new(RecordingPublisher::new());
    let publisher: Arc<dyn EventPublisher> = recorder.clone();

    EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: Some(&outbox_dyn),
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-10"))
    .await
    .expect("the outbox record is the whole of the admission");

    assert_eq!(*rec.lock().expect("recorder poisoned"), ["spool"]);
    assert_eq!(outbox.event_ids(), ["evt-10"]);
}

#[tokio::test]
async fn failed_spool_is_retryable() {
    let outbox: Arc<dyn IngressOutbox> = Arc::new(FakeOutbox::failing());
    let recorder = Arc::new(RecordingPublisher::new());
    let publisher: Arc<dyn EventPublisher> = recorder.clone();
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let events = recording_events(&rec, false);

    let error = EventAdmission {
        task_turns: None,
        task_events: Some(&events),
        ingress_outbox: Some(&outbox),
        publisher: Some(&publisher),
    }
    .admit(message_request("evt-9"))
    .await
    .expect_err("a failed spool is not an admission");

    assert!(error.is_retryable());
    assert!(
        recorder.raw_publishes().is_empty(),
        "an unadmitted payload must not be fanned out"
    );
}

// =========================================================================
// The retry contract
// =========================================================================

#[test]
fn invalid_requests_are_terminal_and_everything_else_retries() {
    let invalid = AdmissionError::Invalid("Unknown event_type: 42".to_string());
    assert!(!invalid.is_retryable());
    let status = invalid.into_status("task-1", "evt-1");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert_eq!(status.message(), "Unknown event_type: 42");

    for retryable in [
        AdmissionError::NotConfigured {
            component: "ingress outbox",
        },
        AdmissionError::CommitFailed {
            what: "turn state",
            source: sqlx::Error::PoolClosed,
        },
        AdmissionError::NoDeliveryPath {
            what: "full agent.message content",
        },
        AdmissionError::AdmissionInFlight,
    ] {
        assert!(retryable.is_retryable());
        let status = retryable.into_status("task-1", "evt-1");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        // Redaction: the stable client response carries no infrastructure
        // detail, only the instruction to retry.
        assert_eq!(
            status.message(),
            "event not durably admitted; retry with the same event_id"
        );
    }
}

// =========================================================================
// Relay
// =========================================================================

#[tokio::test]
async fn relay_republishes_due_records_and_leaves_them_for_their_consumer() {
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let recorder = Arc::new(RecordingPublisher::new());
    let publisher: Arc<dyn EventPublisher> = recorder.clone();

    // Admitted while the transport was down: due immediately.
    outbox
        .admit(
            &IngressPayload {
                event_id: "evt-relay".to_string(),
                task_id: "task-1".to_string(),
                event_type: "agent".to_string(),
                sub_type: "message".to_string(),
                turn_number: Some(1),
                payload: serde_json::json!({ "uuid": "msg-1" }),
                timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            },
            Utc::now() - Duration::seconds(60),
        )
        .await
        .expect("admitted");

    let pass = IngressRelay::new(outbox_dyn.clone(), publisher)
        .relay_once()
        .await;

    assert_eq!(pass.republished, 1);
    assert_eq!(pass.failed, 0);
    let published = recorder.raw_publishes();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].event_id, "evt-relay");
    assert!(!published[0].payload_recoverable);

    // Re-publication is not delivery: the record stays until the consumer
    // commits the message and deletes it.
    assert_eq!(outbox.event_ids(), ["evt-relay"]);
    outbox_dyn
        .delete_delivered("evt-relay")
        .await
        .expect("delete");
    assert!(outbox.event_ids().is_empty());
}

#[tokio::test]
async fn relay_keeps_a_record_whose_republication_failed() {
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher::failing(1));

    outbox
        .admit(
            &IngressPayload {
                event_id: "evt-stuck".to_string(),
                task_id: "task-1".to_string(),
                event_type: "agent".to_string(),
                sub_type: "message".to_string(),
                turn_number: None,
                payload: serde_json::json!({}),
                timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            },
            Utc::now() - Duration::seconds(60),
        )
        .await
        .expect("admitted");

    let pass = IngressRelay::new(outbox_dyn, publisher).relay_once().await;

    assert_eq!(pass.failed, 1);
    assert_eq!(pass.republished, 0);
    assert_eq!(outbox.event_ids(), ["evt-stuck"]);
}

#[tokio::test]
async fn relay_ignores_records_that_are_not_due_yet() {
    let outbox = Arc::new(FakeOutbox::default());
    let outbox_dyn: Arc<dyn IngressOutbox> = outbox.clone();
    let recorder = Arc::new(RecordingPublisher::new());
    let publisher: Arc<dyn EventPublisher> = recorder.clone();

    outbox
        .admit(
            &IngressPayload {
                event_id: "evt-fresh".to_string(),
                task_id: "task-1".to_string(),
                event_type: "agent".to_string(),
                sub_type: "message".to_string(),
                turn_number: None,
                payload: serde_json::json!({}),
                timestamp: "2026-07-12T00:00:00.000Z".to_string(),
            },
            Utc::now() + Duration::seconds(30),
        )
        .await
        .expect("admitted");

    let pass = IngressRelay::new(outbox_dyn, publisher).relay_once().await;

    // The ingress publish normally wins this race; the relay must not double
    // every healthy message onto the stream.
    assert_eq!(pass, super::relay::RelayPass::default());
    assert!(recorder.raw_publishes().is_empty());
}

#[tokio::test]
async fn relay_stops_on_cancellation() {
    let outbox: Arc<dyn IngressOutbox> = Arc::new(FakeOutbox::default());
    let publisher: Arc<dyn EventPublisher> = Arc::new(RecordingPublisher::new());
    let cancel = tokio_util::sync::CancellationToken::new();

    let handle = IngressRelay::new(outbox, publisher).spawn(cancel.clone());
    cancel.cancel();

    // Owned, not detached: the root can observe termination.
    tokio::time::timeout(std::time::Duration::from_secs(5), handle.into_join_handle())
        .await
        .expect("the relay participates in cancellation")
        .expect("and exits cleanly");
}

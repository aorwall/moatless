//! Admission behaviour (the durable-ingress decision), driven through the
//! seams the gRPC ingress actually uses.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::task_event::dao::MockTaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::{SettledTurn, TurnSignals, TurnTransition, UnclosedTurn};
use moatless_core::events::EventPublisher;
use moatless_core::events::test_support::RecordingPublisher;

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
        _ev: &crate::task_turn::models::TurnStateEvent,
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
    ) -> Result<crate::task_turn::models::ForkPoint, sqlx::Error> {
        unreachable!("fork_point is the fork handler's, not the ingress path")
    }

    async fn project_turn(
        &self,
        _task_id: &str,
        _turn_number: i32,
        _agent_profile: Option<String>,
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

    async fn triggered_by_task_id(
        &self,
        _task_id: &str,
        _turn_number: i32,
    ) -> Result<Option<String>, sqlx::Error> {
        Ok(None)
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

// =========================================================================
// Fixtures
// =========================================================================

fn state_request(event_id: &str) -> AdmissionRequest {
    AdmissionRequest {
        event_id: event_id.to_string(),
        task_id: "task-1".to_string(),
        event_type: "taskTurn",
        sub_type: "active".to_string(),
        turn_number: Some(1),
        payload: serde_json::json!({}),
        state: Some(crate::task_turn::models::TurnStateEvent {
            event_id: event_id.to_string(),
            task_id: "task-1".to_string(),
            event_type: "taskTurn".to_string(),
            sub_type: "active".to_string(),
            is_error: false,
            agent_profile: Some("claude-code".to_string()),
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

    EventAdmission::new(Some(turns.clone()), None, Some(publisher.clone()))
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
async fn a_state_event_fans_out_with_the_task_agent_profile() {
    let rec: Recorder = Arc::new(Mutex::new(Vec::new()));
    let turns: Arc<dyn TaskTurnDao> = Arc::new(RecordingTurns {
        rec: rec.clone(),
        fail: false,
    });
    let publisher = Arc::new(RecordingPublisher::new());

    EventAdmission::new(Some(turns), None, Some(publisher.clone()))
        .admit(state_request("evt-5"))
        .await
        .expect("state-bearing event is admitted");

    // The agent handlers label `moatless_agent_*` metrics with this profile.
    let raw = publisher.raw_publishes();
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0].agent_profile.as_deref(), Some("claude-code"));
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

    let error = EventAdmission::new(Some(turns.clone()), None, Some(publisher.clone()))
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

    EventAdmission::new(None, Some(events.clone()), Some(publisher.clone()))
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

    let error = EventAdmission::new(None, Some(events.clone()), Some(publisher.clone()))
        .admit(plain_request("evt-4"))
        .await
        .expect_err("a failed row commit is not an admission");

    assert!(error.is_retryable());
    assert_eq!(*rec.lock().expect("recorder poisoned"), ["event-row"]);
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
            component: "task event repository",
        },
        AdmissionError::CommitFailed {
            what: "turn state",
            source: sqlx::Error::PoolClosed,
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

#[test]
fn a_stopped_recording_is_stored_as_an_error() {
    assert!(super::is_error_sub_type("recording_stopped"));
    assert!(super::is_error_sub_type("failed"));
    assert!(!super::is_error_sub_type("context_window"));
}

//! Why a task's previous sandbox container stopped, for the restart note.

use tonic::{Request, Response, Status};
use tracing::warn;

use moatless_tasks::task_event::models::TaskEvent;
use sandbox_proto::{GetLastStopCauseRequest, GetLastStopCauseResponse, StopCause};

use super::state::BackendSandboxServiceImpl;

/// Lifecycle events that end a container. `startup_slow`, `pod_scheduled` and
/// the like describe the replacement and are passed over.
const STOP_EVENTS: &[&str] = &[
    "container_oom_killed",
    "container_restarted",
    "container_terminated",
    "container_crash_loop",
    "pod_evicted",
    "pod_failed",
    "node_reclaimed",
    "pod_deleted",
];

/// How many of the newest `sandbox` events are searched. The cause lands just
/// before the replacement's own events, and the previous `pod_ready` ends the
/// search well inside this.
const SEARCH_DEPTH: i64 = 50;

pub(super) async fn get_last_stop_cause(
    service: &BackendSandboxServiceImpl,
    request: Request<GetLastStopCauseRequest>,
) -> Result<Response<GetLastStopCauseResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;

    let task_events = service
        .ingress
        .as_ref()
        .map(|i| i.task_events.clone())
        .ok_or_else(|| Status::failed_precondition("Database not configured"))?;

    let events = task_events
        .list_recent_events(
            &task_id,
            &Some(vec!["sandbox".to_string()]),
            SEARCH_DEPTH,
            0,
        )
        .await
        .map_err(|error| {
            warn!(task_id, %error, "failed to read the sandbox events for the stop cause");
            Status::internal("Failed to read the sandbox events")
        })?;

    Ok(Response::new(GetLastStopCauseResponse {
        cause: last_stop_cause(events.iter().rev()),
    }))
}

/// The newest stop recorded after the previous container's `pod_ready`, from
/// events given newest first.
///
/// Only correct while the caller's own container has not reported Ready: its
/// `pod_ready` would otherwise end the search before the cause.
fn last_stop_cause<'a>(newest_first: impl Iterator<Item = &'a TaskEvent>) -> Option<StopCause> {
    for event in newest_first {
        let payload = &event.payload;
        let field = |name: &str| {
            payload
                .get(name)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let lifecycle_event = field("lifecycleEvent");
        let kind = if event.sub_type == "stopped" {
            "stopped".to_string()
        } else {
            match lifecycle_event {
                Some(ref name) if name == "pod_ready" => return None,
                Some(name) if STOP_EVENTS.contains(&name.as_str()) => name,
                _ => continue,
            }
        };
        return Some(StopCause {
            kind,
            terminated_reason: field("terminatedReason"),
            exit_code: payload
                .get("exitCode")
                .and_then(serde_json::Value::as_i64)
                .and_then(|code| i32::try_from(code).ok()),
            disruption_reason: field("disruptionReason"),
            initiator: field("initiator"),
            occurred_at_unix_ms: event.created_at.timestamp_millis(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(sub_type: &str, payload: serde_json::Value) -> TaskEvent {
        TaskEvent {
            id: 0,
            event_id: String::new(),
            task_id: "task-1".to_string(),
            event_type: "sandbox".to_string(),
            sub_type: sub_type.to_string(),
            is_error: false,
            agent_profile: None,
            turn_number: None,
            payload,
            timestamp: String::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// The incident's order, newest first: the replacement's init events land
    /// around the kill that preceded them.
    #[test]
    fn the_kill_after_the_previous_ready_is_the_cause() {
        let events = [
            event("initializing", json!({"reason": "RepositoryRestored"})),
            event(
                "initializing",
                json!({"lifecycleEvent": "startup_slow", "disruptionReason": "Unhealthy"}),
            ),
            event(
                "initializing",
                json!({"lifecycleEvent": "container_oom_killed", "exitCode": 137, "terminatedReason": "OOMKilled"}),
            ),
            event("workspace_synced", json!({"ok": true})),
            event("ready", json!({"lifecycleEvent": "pod_ready"})),
        ];

        let cause = last_stop_cause(events.iter()).expect("a cause");

        assert_eq!(cause.kind, "container_oom_killed");
        assert_eq!(cause.exit_code, Some(137));
        assert_eq!(cause.terminated_reason.as_deref(), Some("OOMKilled"));
    }

    /// A kill older than the previous container's Ready belongs to an earlier
    /// restart, so it says nothing about this one.
    #[test]
    fn a_kill_before_the_previous_ready_is_not_the_cause() {
        let events = [
            event("workspace_synced", json!({"ok": true})),
            event("ready", json!({"lifecycleEvent": "pod_ready"})),
            event(
                "initializing",
                json!({"lifecycleEvent": "container_restarted", "exitCode": 137}),
            ),
        ];

        assert_eq!(last_stop_cause(events.iter()), None);
    }

    #[test]
    fn a_deliberate_stop_names_who_stopped_it() {
        let events = [event(
            "stopped",
            json!({"initiator": "reaper", "lifecycleEvent": "idle_reaped"}),
        )];

        let cause = last_stop_cause(events.iter()).expect("a cause");

        assert_eq!(cause.kind, "stopped");
        assert_eq!(cause.initiator.as_deref(), Some("reaper"));
    }
}

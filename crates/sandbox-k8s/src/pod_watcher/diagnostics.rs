//! Best-effort reads that name the cause of a failure: container logs, the
//! pod's own K8s events, and the turn an interruption cuts short.

use super::*;

/// Fetch the last N lines of the sandbox container's logs.
/// Returns `None` if logs can't be fetched (best-effort, never fails).
pub(super) async fn fetch_sandbox_logs(
    k8s: &K8sClient,
    pod_name: &str,
    tail_lines: i64,
) -> Option<String> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let params = LogParams {
        container: Some("sandbox".to_string()),
        tail_lines: Some(tail_lines),
        ..Default::default()
    };
    match pods_api.logs(pod_name, &params).await {
        Ok(logs) if !logs.is_empty() => Some(logs),
        Ok(_) => None,
        Err(e) => {
            debug!(pod_name = %pod_name, error = %e, "Failed to fetch sandbox logs for root cause");
            None
        }
    }
}

/// Best-effort lookup of the most relevant K8s Event reason+message for a pod.
/// Names the cause of a stuck-Pending / slow-start window (e.g.
/// `FailedScheduling`, `FailedAttachVolume`, `Multi-Attach error for volume …`).
///
/// Lists core/v1 Events with a field selector on `involvedObject.name`, prefers
/// the last `Warning`-type event, and falls back to the last event of any type.
/// Returns `None` when nothing is found or the API call fails (e.g. missing RBAC
/// for events) — the caller degrades to emitting the event without a cause chip.
pub(super) async fn fetch_pod_event_cause(k8s: &K8sClient, pod_name: &str) -> Option<String> {
    let events_api: Api<CoreEvent> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().fields(&format!("involvedObject.name={pod_name}"));
    let list = match events_api.list(&lp).await {
        Ok(list) => list,
        Err(e) => {
            debug!(pod_name = %pod_name, error = %e, "Failed to list pod events for cause");
            return None;
        }
    };

    // Prefer the newest Warning; fall back to the newest event of any type. K8s
    // orders list results arbitrarily, so pick by `last_timestamp` explicitly.
    let newest_warning = list
        .items
        .iter()
        .filter(|e| e.type_.as_deref() == Some("Warning"))
        .max_by_key(|e| e.last_timestamp.as_ref().map(|t| t.0));
    let event = match newest_warning {
        Some(e) => e,
        None => list
            .items
            .iter()
            .max_by_key(|e| e.last_timestamp.as_ref().map(|t| t.0))?,
    };

    let reason = event.reason.as_deref().unwrap_or("").trim();
    let message = event.message.as_deref().unwrap_or("").trim();
    match (reason.is_empty(), message.is_empty()) {
        (true, true) => None,
        (false, true) => Some(reason.to_string()),
        (true, false) => Some(message.to_string()),
        (false, false) => Some(format!("{reason}: {message}")),
    }
}

/// Emit a `taskTurn` `turn_interrupted` event. The event is SSE-only, so it
/// carries no turn context: the eventual terminal event / reaper is what closes
/// the turn.
pub(super) async fn emit_turn_interrupted(
    event_publisher: &dyn EventPublisher,
    task_id: &str,
    cause: Option<&str>,
) {
    let message = match cause {
        Some(c) => format!(
            "Turn interrupted — sandbox pod terminated involuntarily ({c}). Resuming when it reschedules."
        ),
        None => {
            "Turn interrupted — sandbox pod terminated involuntarily. Resuming when it reschedules."
                .to_string()
        }
    };
    let mut payload = serde_json::json!({ "message": message });
    if let Some(c) = cause {
        payload["reason"] = serde_json::Value::String(c.to_string());
    }

    info!(
        task_id = %task_id,
        cause = ?cause,
        "Emitting turn_interrupted"
    );
    event_publisher
        .publish_task_turn_event(task_id, TaskTurnSubType::Interrupted, None, None, payload)
        .await;
}

//! Which Sandbox events the T3 setup card shows instead of the transcript.

use serde_json::Value;

/// Whether a Sandbox event is one step of a start going as planned: the
/// acceptance, scheduling, each in-sandbox init step, and the ready signal.
///
/// The T3 setup card shows these, so the transcript does not; a warning such
/// as `CredentialsFailed` keeps its row, as does every other lifecycle event.
pub fn is_start_step(sub_type: &str, payload: &Value) -> bool {
    let lifecycle_event = payload
        .get("lifecycleEvent")
        .and_then(Value::as_str)
        .filter(|event| !event.is_empty());
    let warning = payload.get("eventType").and_then(Value::as_str) == Some("Warning");
    match sub_type {
        "initializing" => !warning && matches!(lifecycle_event, None | Some("pod_scheduled")),
        "ready" => matches!(lifecycle_event, None | Some("pod_ready")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The steps of a start belong to the setup card; a warning during one,
    /// and every stop or failure, stay in the transcript.
    #[test]
    fn only_the_steps_of_a_start_going_as_planned_are_start_steps() {
        let steps = [
            ("initializing", json!({ "message": "Sandbox initializing" })),
            ("initializing", json!({ "lifecycleEvent": "pod_scheduled" })),
            (
                "initializing",
                json!({ "reason": "CredentialsFetched", "eventType": "Normal" }),
            ),
            ("ready", json!({ "lifecycleEvent": "pod_ready" })),
        ];
        for (sub_type, payload) in &steps {
            assert!(is_start_step(sub_type, payload), "{sub_type} {payload}");
        }
        let kept = [
            (
                "initializing",
                json!({ "reason": "CredentialsFailed", "eventType": "Warning" }),
            ),
            ("initializing", json!({ "lifecycleEvent": "startup_slow" })),
            (
                "initializing",
                json!({ "lifecycleEvent": "container_restarted" }),
            ),
            ("stopped", json!({ "message": "Sandbox stopped" })),
            ("error", json!({ "message": "Sandbox start failed" })),
        ];
        for (sub_type, payload) in &kept {
            assert!(!is_start_step(sub_type, payload), "{sub_type} {payload}");
        }
    }
}

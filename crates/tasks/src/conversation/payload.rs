//! The `task_events` payloads a conversation reads, decoded in one place for
//! the Projector and every reader.

use serde_json::{Map, Value, json};

/// What a system Message's event says about the run it ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ResultMeta {
    pub will_not_resume: Option<bool>,
    pub run_metrics: Option<Value>,
}

/// Decodes `payload` of the `task.error` or `agent.completed` event a system
/// Message names; `sub_type` picks which.
pub(crate) fn result_meta(sub_type: &str, payload: &Value) -> ResultMeta {
    if sub_type == moatless_types::task::TaskSubType::Error.as_str() {
        return ResultMeta {
            will_not_resume: Some(false),
            run_metrics: None,
        };
    }
    let metrics: Map<String, Value> = [
        ("totalCostUsd", "total_cost_usd"),
        ("numTurns", "num_turns"),
        ("durationMs", "duration_ms"),
        ("durationApiMs", "duration_api_ms"),
        ("model", "model"),
    ]
    .into_iter()
    .filter_map(|(key, alias)| {
        let value = payload.get(key).or_else(|| payload.get(alias))?;
        (!value.is_null()).then(|| (key.to_string(), value.clone()))
    })
    .collect();
    ResultMeta {
        will_not_resume: payload.get("willNotResume").and_then(Value::as_bool),
        run_metrics: (!metrics.is_empty()).then_some(Value::Object(metrics)),
    }
}

/// Whether an `agent.completed` payload reports a failed run, which writes no
/// turn result: its `agent.error` says what happened.
pub(crate) fn is_error_result(payload: &Value) -> bool {
    payload
        .get("isError")
        .or_else(|| payload.get("is_error"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// A `task.binding_reply_sent` payload.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BindingReply {
    pub text: String,
    /// The agent's reply, which the conversation shows; a stub announcing a
    /// run is only the Binding's.
    pub shown: bool,
}

impl BindingReply {
    pub(crate) fn of(payload: &Value) -> Option<Self> {
        Some(Self {
            text: payload.get("text")?.as_str()?.to_string(),
            shown: payload.get("kind").and_then(Value::as_str) == Some(BINDING_REPLY),
        })
    }

    /// The payload a `task.binding_reply_sent` event carries.
    pub(crate) fn payload(text: &str, shown: bool, provider: Option<&str>) -> Value {
        json!({
            "kind": if shown { BINDING_REPLY } else { BINDING_STUB },
            "text": text,
            "provider": provider,
        })
    }
}

const BINDING_REPLY: &str = "reply";
const BINDING_STUB: &str = "stub";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the payload shapes the writers emit: `agent.completed` is the SDK
    /// result as the sandbox forwards it, in either casing.
    #[test]
    fn decodes_the_payloads_writers_emit() {
        let completed = json!({
            "sessionId": "s", "isError": false, "subtype": "success", "result": "done",
            "total_cost_usd": 0.25, "numTurns": 3, "durationMs": 1200, "duration_api_ms": 900,
        });
        assert_eq!(
            result_meta("completed", &completed),
            ResultMeta {
                will_not_resume: None,
                run_metrics: Some(json!({
                    "totalCostUsd": 0.25, "numTurns": 3, "durationMs": 1200, "durationApiMs": 900,
                })),
            }
        );
        assert!(!is_error_result(&completed));
        assert!(is_error_result(&json!({ "is_error": true })));
        assert_eq!(
            result_meta("error", &json!({ "text": "boom" })),
            ResultMeta {
                will_not_resume: Some(false),
                run_metrics: None
            }
        );
        assert_eq!(result_meta("completed", &json!({})).run_metrics, None);

        let reply = BindingReply::payload("hi", true, Some("slack"));
        assert_eq!(
            BindingReply::of(&reply),
            Some(BindingReply {
                text: "hi".into(),
                shown: true
            })
        );
        let stub = BindingReply::payload("started", false, None);
        assert_eq!(BindingReply::of(&stub).map(|r| r.shown), Some(false));
    }
}

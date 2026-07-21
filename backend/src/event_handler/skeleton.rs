//! One shared projection reducing an `agent.message` source event to its
//! persisted-and-relayed **metadata skeleton** (ADR-0003).
//!
//! A Message's content is written once, to `task_messages`. The full
//! `agent.message` content is a **transient ingestion payload**: the Sandbox
//! emits it to the Redis stream, the event-handler consumer reads it once to
//! build `task_messages`, and it is never persisted to `task_events` nor relayed
//! to clients in full. This projection produces the small metadata form that
//! *is* persisted and relayed:
//!
//! ```json
//! { "message": { "role": "assistant" }, "uuid": "…", "blockCount": 3, "toolNames": ["Bash", "Edit"] }
//! ```
//!
//! `role` stays nested under `message` so the timeline's existing role reader
//! works for both old and new rows; `toolNames` is the source-event signal the
//! files-invalidation reaction reads (a field, not content).
//!
//! It is applied at every place the event is written to `task_events` or
//! broadcast to clients, and **nowhere** on the Redis ingestion path the
//! consumer reads to build the Message store.

use serde_json::{json, Value};

/// Project an `agent.message` **inner payload** (the `AgentEventPayload` body:
/// `{ message: { role, content }, uuid, … }`) to its metadata skeleton.
/// Non-object inputs and payloads without a `message` are returned as an empty
/// skeleton so a malformed row never keeps its heavy content.
pub fn agent_message_skeleton(payload: &Value) -> Value {
    let message = payload.get("message");
    let role = message
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        .unwrap_or("assistant");

    let content = message.and_then(|m| m.get("content"));
    let (block_count, tool_names) = match content.and_then(Value::as_array) {
        Some(blocks) => {
            let names: Vec<String> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter_map(|b| {
                    b.get("name")
                        .and_then(Value::as_str)
                        .map(std::string::ToString::to_string)
                })
                .collect();
            (blocks.len(), names)
        }
        None => (0usize, Vec::new()),
    };

    json!({
        "message": { "role": role },
        "uuid": payload.get("uuid").cloned().unwrap_or(Value::Null),
        "blockCount": block_count,
        "toolNames": tool_names,
    })
}

/// Whether an event `(type, sub_type)` is an `agent.message` source event — the
/// only event this projection slims. The derived `message.upsert` /
/// `subagentProgress` events (`type = "message"`) are the Chat's content
/// delivery and are never slimmed.
pub fn is_agent_message(event_type: &str, sub_type: &str) -> bool {
    event_type == "agent" && sub_type == "message"
}

/// Slim a serialized `StreamEvent` envelope for **relay to clients**: when it is
/// an `agent.message`, replace its inner `payload` with the skeleton and
/// re-serialize; otherwise return `None` so the caller relays the raw string
/// untouched (the common, allocation-free path).
pub fn slim_relayed_event(raw: &str) -> Option<String> {
    let mut envelope: Value = serde_json::from_str(raw).ok()?;
    let obj = envelope.as_object_mut()?;
    let event_type = obj.get("type").and_then(Value::as_str)?;
    let sub_type = obj.get("subType").and_then(Value::as_str)?;
    if !is_agent_message(event_type, sub_type) {
        return None;
    }
    let skeleton = agent_message_skeleton(obj.get("payload").unwrap_or(&Value::Null));
    obj.insert("payload".to_string(), skeleton);
    serde_json::to_string(&envelope).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_role_block_count_and_tool_names() {
        let payload = json!({
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "hi" },
                    { "type": "tool_use", "id": "t1", "name": "Bash", "input": {} },
                    { "type": "tool_use", "id": "t2", "name": "Edit", "input": {} },
                ]
            },
            "uuid": "evt-1",
            "session_id": "sess-1"
        });
        let skeleton = agent_message_skeleton(&payload);
        assert_eq!(skeleton["message"]["role"], "assistant");
        assert_eq!(skeleton["uuid"], "evt-1");
        assert_eq!(skeleton["blockCount"], 3);
        assert_eq!(skeleton["toolNames"], json!(["Bash", "Edit"]));
        // Heavy content is dropped.
        assert!(skeleton["message"].get("content").is_none());
        assert!(skeleton.get("session_id").is_none());
    }

    #[test]
    fn empty_skeleton_for_missing_message() {
        let skeleton = agent_message_skeleton(&json!({ "uuid": "e" }));
        assert_eq!(skeleton["message"]["role"], "assistant");
        assert_eq!(skeleton["blockCount"], 0);
        assert_eq!(skeleton["toolNames"], json!([]));
    }

    #[test]
    fn slim_relayed_event_only_touches_agent_message() {
        let agent = json!({
            "type": "agent",
            "subType": "message",
            "taskId": "task-1",
            "payload": { "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "t1", "name": "Read", "input": {} }
            ] }, "uuid": "u1" }
        })
        .to_string();
        let slimmed = slim_relayed_event(&agent).expect("agent.message is slimmed");
        let parsed: Value = serde_json::from_str(&slimmed).unwrap();
        assert_eq!(parsed["payload"]["blockCount"], 1);
        assert_eq!(parsed["payload"]["toolNames"], json!(["Read"]));
        assert!(parsed["payload"]["message"].get("content").is_none());

        // A derived message.upsert is left untouched.
        let upsert = json!({
            "type": "message",
            "subType": "upsert",
            "taskId": "task-1",
            "payload": { "uuid": "u1", "content": [{ "type": "text", "text": "hi" }] }
        })
        .to_string();
        assert!(slim_relayed_event(&upsert).is_none());
    }
}

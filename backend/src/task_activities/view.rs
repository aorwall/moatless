//! Canonical read-side meaning of one stored activity.
//!
//! Client adapters should translate this view into their own wire shape instead
//! of re-reading storage payload rules locally.

use serde_json::{Map, Value};

use super::dao::Activity;
use super::mapping::{ActivityKind, ActivityStatus, result_text};

pub struct ActivityView<'a> {
    activity: &'a Activity,
}

impl<'a> ActivityView<'a> {
    pub fn new(activity: &'a Activity) -> Self {
        Self { activity }
    }

    pub fn activity_id(&self) -> &str {
        &self.activity.activity_id
    }

    pub fn kind(&self) -> ActivityKind {
        self.activity.kind
    }

    pub fn status(&self) -> ActivityStatus {
        self.activity.status
    }

    pub fn summary(&self) -> &str {
        &self.activity.summary
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.activity.tool_use_id.as_deref()
    }

    pub fn input(&self) -> &Value {
        self.activity.tool_input()
    }

    pub fn result_text(&self) -> Option<&str> {
        self.activity.result().and_then(result_text)
    }

    pub fn typed_result(&self) -> Option<&Value> {
        self.activity.typed_result()
    }

    pub fn detail(&self) -> Option<&str> {
        self.activity.detail.as_deref()
    }

    pub fn is_error(&self) -> bool {
        self.activity.is_error
    }

    pub fn is_settled(&self) -> bool {
        self.activity.is_settled()
    }

    pub fn t3_tone(&self) -> &'static str {
        match self.activity.kind {
            _ if self.activity.is_error => "error",
            ActivityKind::ToolCall => "tool",
            ActivityKind::Thinking
            | ActivityKind::SandboxStatus
            | ActivityKind::SandboxError
            | ActivityKind::UserInputRequested
            | ActivityKind::UserInputResolved
            | ActivityKind::PlanUpdated
            | ActivityKind::ContextWindowUpdated => "info",
        }
    }

    pub fn t3_kind(&self) -> &'static str {
        match self.activity.kind {
            ActivityKind::ToolCall if self.activity.is_settled() => "tool.completed",
            ActivityKind::ToolCall => "tool.updated",
            ActivityKind::Thinking | ActivityKind::SandboxStatus | ActivityKind::SandboxError => {
                "task.progress"
            }
            // The four the client derives a panel from rather than a work-log
            // row. Each spelling is the one that client's own deriver matches
            // on, which is why they are dotted where the column is not.
            ActivityKind::UserInputRequested => "user-input.requested",
            ActivityKind::UserInputResolved => "user-input.resolved",
            ActivityKind::PlanUpdated => "turn.plan.updated",
            ActivityKind::ContextWindowUpdated => "context-window.updated",
        }
    }

    pub fn t3_payload(&self) -> Map<String, Value> {
        let mut payload = match &self.activity.payload {
            Value::Object(fields) => fields.clone(),
            _ => Map::new(),
        };
        payload.remove("input");
        payload.remove("result");
        payload.remove("typedResult");
        if let Some(detail) = self.detail() {
            payload.insert("detail".into(), Value::String(detail.to_string()));
        }
        payload
    }

    pub fn external_adapter(&self) -> Option<&str> {
        self.activity.external_adapter.as_deref()
    }

    pub fn external_event_id(&self) -> Option<&str> {
        self.activity.external_event_id.as_deref()
    }

    pub fn external_source_name(&self) -> Option<&str> {
        self.activity.external_source_name.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn stored(kind: ActivityKind, payload: Value) -> Activity {
        Activity {
            activity_id: "msg-1:0".to_string(),
            task_id: "task-1".to_string(),
            position: 1,
            ordinal: 0,
            message_uuid: Some("msg-1".to_string()),
            parent_uuid: None,
            kind,
            status: ActivityStatus::Completed,
            tool_use_id: None,
            summary: "summary".to_string(),
            payload,
            detail: None,
            is_error: false,
            external_event_id: None,
            external_adapter: None,
            external_source_name: None,
            created_at: Utc::now(),
        }
    }

    /// Each spelling is the one the client's own deriver matches on. A kind that
    /// reaches it under any other name renders as nothing at all.
    #[test]
    fn every_kind_reaches_the_client_under_the_name_it_matches_on() {
        for (kind, expected) in [
            (ActivityKind::Thinking, "task.progress"),
            (ActivityKind::SandboxStatus, "task.progress"),
            (ActivityKind::SandboxError, "task.progress"),
            (ActivityKind::UserInputRequested, "user-input.requested"),
            (ActivityKind::UserInputResolved, "user-input.resolved"),
            (ActivityKind::PlanUpdated, "turn.plan.updated"),
            (ActivityKind::ContextWindowUpdated, "context-window.updated"),
        ] {
            let row = stored(kind, json!({}));
            assert_eq!(ActivityView::new(&row).t3_kind(), expected);
        }

        let mut call = stored(ActivityKind::ToolCall, json!({}));
        call.status = ActivityStatus::InProgress;
        assert_eq!(ActivityView::new(&call).t3_kind(), "tool.updated");
        call.status = ActivityStatus::Completed;
        assert_eq!(ActivityView::new(&call).t3_kind(), "tool.completed");
    }

    /// A tool call's raw input and result never reach this contract — it
    /// re-sends its whole snapshot every time the conversation moves. The rows
    /// a panel is derived from carry their own payload instead, which is why
    /// the questions survive the strip.
    #[test]
    fn a_panels_payload_survives_the_strip_a_calls_input_does_not() {
        let call = stored(
            ActivityKind::ToolCall,
            json!({ "title": "AskUserQuestion", "input": { "questions": [] }, "result": "ok" }),
        );
        let stripped = ActivityView::new(&call).t3_payload();
        assert!(stripped.get("input").is_none());
        assert!(stripped.get("result").is_none());

        let asked = stored(
            ActivityKind::UserInputRequested,
            json!({
                "requestId": "tu-1",
                "questions": [{ "id": "Which?", "question": "Which?" }]
            }),
        );
        let payload = ActivityView::new(&asked).t3_payload();
        assert_eq!(payload["requestId"], "tu-1");
        assert_eq!(payload["questions"][0]["id"], "Which?");
    }

    /// The four are things that happened, not things that failed.
    #[test]
    fn a_panel_row_is_toned_as_information() {
        for kind in [
            ActivityKind::UserInputRequested,
            ActivityKind::UserInputResolved,
            ActivityKind::PlanUpdated,
            ActivityKind::ContextWindowUpdated,
        ] {
            let row = stored(kind, json!({}));
            assert_eq!(ActivityView::new(&row).t3_tone(), "info");
        }
    }
}

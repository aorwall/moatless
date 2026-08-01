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
            ActivityKind::Thinking | ActivityKind::SandboxStatus | ActivityKind::SandboxError => {
                "info"
            }
        }
    }

    pub fn t3_kind(&self) -> &'static str {
        match self.activity.kind {
            ActivityKind::ToolCall if self.activity.is_settled() => "tool.completed",
            ActivityKind::ToolCall => "tool.updated",
            ActivityKind::Thinking | ActivityKind::SandboxStatus | ActivityKind::SandboxError => {
                "task.progress"
            }
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

use super::super::content::CanonicalContentBlock as Block;
use super::super::source::Role;
use super::super::trajectory::{EventContent, RecordedLine, RenderContext, SessionFormat, Usage};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Pi's append-only version-three session tree.
pub(crate) struct PiSessionFormat;

pub(super) fn blocks(message: &Value) -> Vec<Block> {
    if message["role"] == "toolResult" {
        return vec![Block::ToolResult {
            tool_use_id: message["toolCallId"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            content: Some(message["content"].clone()),
            is_error: message["isError"].as_bool(),
        }];
    }
    if let Some(text) = message["content"].as_str() {
        return vec![Block::Text { text: text.into() }];
    }
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| match b["type"].as_str() {
            Some("text") => Block::Text {
                text: b["text"].as_str().unwrap_or_default().into(),
            },
            Some("thinking") => Block::Thinking {
                thinking: b["thinking"].as_str().unwrap_or_default().into(),
                signature: b["thinkingSignature"].as_str().unwrap_or_default().into(),
            },
            Some("toolCall") => Block::ToolUse {
                id: b["id"].as_str().unwrap_or_default().into(),
                name: b["name"].as_str().unwrap_or_default().into(),
                input: b["arguments"].clone(),
            },
            _ => Block::Unknown { raw: b.clone() },
        })
        .collect()
}

impl SessionFormat for PiSessionFormat {
    fn harness(&self) -> &'static str {
        moatless_types::agent::Harness::Pi.as_str()
    }
    fn session_path(&self, home: &Path, _cwd: &Path, id: &str, _first: &str) -> PathBuf {
        home.join("sessions").join(format!("{id}.jsonl"))
    }
    fn locate(&self, home: &Path, cwd: &Path, id: &str) -> Option<PathBuf> {
        let path = self.session_path(home, cwd, id, "");
        path.is_file().then_some(path)
    }
    fn latest(&self, _home: &Path, _cwd: &Path) -> Option<String> {
        None
    }
    fn retarget(&self, payload: &str, id: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
            return payload.into();
        };
        if value["type"] == "session" {
            value["id"] = json!(id);
            if let Some(object) = value.as_object_mut() {
                object.remove("parentSession");
            }
        }
        if value["type"] == "session" {
            value.to_string()
        } else {
            payload.to_string()
        }
    }
    fn read_line(&self, line: &str) -> RecordedLine {
        let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let message = &value["message"];
        if value["type"] == "compaction" {
            return RecordedLine {
                kind: "compaction".into(),
                harness_item_id: value["id"].as_str().map(str::to_string),
                harness_turn_id: None,
                content: EventContent {
                    role: Some(Role::User),
                    blocks: vec![Block::Text {
                        text: value["summary"].as_str().unwrap_or_default().into(),
                    }],
                    compaction: true,
                    ..EventContent::default()
                },
                derived: false,
                sets_model: None,
                quotes_reply: false,
            };
        }
        let role = match message["role"].as_str() {
            Some("assistant") => Some(Role::Assistant),
            Some("user" | "toolResult") => Some(Role::User),
            Some("system") => Some(Role::System),
            _ => None,
        };
        let usage = &message["usage"];
        RecordedLine {
            kind: value["type"].as_str().unwrap_or("unparsed").into(),
            harness_item_id: value["id"].as_str().map(str::to_string),
            harness_turn_id: None,
            content: EventContent {
                role,
                blocks: blocks(message),
                injected: role == Some(Role::System),
                model: message["model"].as_str().map(|m| {
                    format!(
                        "{}/{}",
                        message["provider"].as_str().unwrap_or("unknown"),
                        m
                    )
                }),
                usage: usage.is_object().then(|| Usage {
                    input_tokens: usage["input"].as_u64().unwrap_or(0),
                    output_tokens: usage["output"].as_u64().unwrap_or(0),
                    cache_read_tokens: usage["cacheRead"].as_u64().unwrap_or(0),
                    cache_write_tokens: usage["cacheWrite"].as_u64().unwrap_or(0),
                }),
                ends_turn: message["stopReason"] == "stop",
                api_error: message["stopReason"] == "error",
                ..EventContent::default()
            },
            derived: false,
            sets_model: None,
            quotes_reply: false,
        }
    }
    fn render(&self, items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String> {
        let mut lines = Vec::new();
        if ctx.prior.is_empty() {
            lines.push(json!({"type":"session","version":3,"id":ctx.harness_session_id,"timestamp":ctx.timestamp,"cwd":ctx.cwd}).to_string());
        }
        let mut parent = ctx
            .prior
            .iter()
            .rev()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .filter(|v| v["type"] != "session")
            .find_map(|v| v["id"].as_str().map(str::to_string));
        for (index, item) in items.iter().enumerate() {
            if item.injected || item.derived_from.is_some() {
                continue;
            }
            let text = item
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Text { text } => Some(text.clone()),
                    Block::ToolUse { name, input, .. } => Some(format!("Tool {name}: {input}")),
                    Block::ToolResult { content, .. } => Some(format!(
                        "Tool result: {}",
                        content.as_ref().unwrap_or(&Value::Null)
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                continue;
            }
            let id = uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("{}:{}:{index}", ctx.harness_session_id, ctx.prior.len()).as_bytes(),
            )
            .to_string();
            let mut message =
                json!({"role":"user","content":[{"type":"text","text":text}],"timestamp":0});
            if item.role == Some(Role::Assistant) {
                let (provider, model) = ctx
                    .model
                    .and_then(|m| m.split_once('/'))
                    .unwrap_or(("anthropic", "imported"));
                message["role"] = json!("assistant");
                message["provider"] = json!(provider);
                message["model"] = json!(model);
                message["api"] = json!(if provider == "openai" {
                    "openai-responses"
                } else {
                    "anthropic-messages"
                });
                message["stopReason"] = json!("stop");
                message["usage"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}});
            }
            lines.push(json!({"type":"message","id":id,"parentId":parent,"timestamp":ctx.timestamp,"message":message}).to_string());
            parent = Some(id);
        }
        lines
    }
}

use super::super::content::CanonicalContentBlock as Block;
use super::super::source::Role;
use super::super::trajectory::{EventContent, RecordedLine, RenderContext, SessionFormat, Usage};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) struct MiniSweSessionFormat;

pub(super) fn blocks(message: &Value) -> Vec<Block> {
    if message["role"] == "tool" || message["type"] == "function_call_output" {
        return vec![Block::ToolResult {
            tool_use_id: message["tool_call_id"]
                .as_str()
                .or_else(|| message["call_id"].as_str())
                .unwrap_or_default()
                .into(),
            content: Some(if message["role"] == "tool" {
                message["content"].clone()
            } else {
                message["output"].clone()
            }),
            is_error: message["extra"]["returncode"].as_i64().map(|c| c != 0),
        }];
    }
    let mut result = Vec::new();
    if let Some(text) = message["content"].as_str().filter(|s| !s.is_empty()) {
        result.push(Block::Text { text: text.into() });
    }
    for item in message["content"].as_array().into_iter().flatten() {
        if let Some(text) = item["text"].as_str() {
            result.push(Block::Text { text: text.into() });
        }
    }
    for item in message["output"].as_array().into_iter().flatten() {
        if item["type"] == "message" {
            result.extend(blocks(item));
        }
    }
    for action in message["extra"]["actions"].as_array().into_iter().flatten() {
        result.push(Block::ToolUse {
            id: action["tool_call_id"].as_str().unwrap_or_default().into(),
            name: "bash".into(),
            input: json!({"command":action["command"]}),
        });
    }
    result
}

pub(super) fn usage(message: &Value) -> Usage {
    let u = if message["object"] == "response" {
        &message["usage"]
    } else {
        &message["extra"]["response"]["usage"]
    };
    let cached = u["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .or_else(|| u["prompt_tokens_details"]["cached_tokens"].as_u64())
        .unwrap_or(0);
    Usage {
        input_tokens: u["input_tokens"]
            .as_u64()
            .or_else(|| u["prompt_tokens"].as_u64())
            .unwrap_or(0)
            .saturating_sub(cached),
        output_tokens: u["output_tokens"]
            .as_u64()
            .or_else(|| u["completion_tokens"].as_u64())
            .unwrap_or(0),
        cache_read_tokens: cached,
        cache_write_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
    }
}

impl SessionFormat for MiniSweSessionFormat {
    fn harness(&self) -> &'static str {
        moatless_types::agent::Harness::MiniSwe.as_str()
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
        let Ok(mut v) = serde_json::from_str::<Value>(payload) else {
            return payload.into();
        };
        if v["type"] == "session" {
            v["id"] = json!(id);
            v.to_string()
        } else {
            payload.into()
        }
    }
    fn read_line(&self, line: &str) -> RecordedLine {
        let v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let m = &v["message"];
        let role = match m["role"].as_str() {
            Some("assistant" | "exit") => Some(Role::Assistant),
            Some("user" | "tool") => Some(Role::User),
            Some("system") => Some(Role::System),
            _ if m["object"] == "response" => Some(Role::Assistant),
            _ if m["type"] == "function_call_output" => Some(Role::User),
            _ => None,
        };
        RecordedLine {
            kind: v["type"].as_str().unwrap_or("unparsed").into(),
            harness_item_id: v["id"].as_str().map(str::to_string),
            harness_turn_id: None,
            content: EventContent {
                role,
                blocks: blocks(m),
                injected: role == Some(Role::System),
                model: v["model"].as_str().map(str::to_string),
                usage: (m["object"] == "response" || m["extra"]["response"].is_object())
                    .then(|| usage(m)),
                ends_turn: m["role"] == "exit" || v["ends_turn"] == true,
                api_error: m["role"] == "exit" && m["extra"]["exit_status"] != "Submitted",
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
            lines.push(
                json!({"type":"session","version":1,"id":ctx.harness_session_id}).to_string(),
            );
        }
        for item in items {
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
            let role = if item.role == Some(Role::Assistant) {
                "assistant"
            } else {
                "user"
            };
            lines.push(
                json!({"type":"message","id":uuid::Uuid::new_v4().to_string(),"model":ctx.model,"ends_turn":item.ends_turn,
                "message":{"role":role,"content":text}})
                .to_string(),
            );
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_keep_tool_ids_usage_and_submission_boundary() {
        let response = json!({"type":"message","id":"r1","model":"openai/test","message":{
            "object":"response","usage":{"input_tokens":120,"output_tokens":20,"input_tokens_details":{"cached_tokens":100}},
            "output":[{"type":"message","content":[{"type":"output_text","text":"Working"}]}],
            "extra":{"actions":[{"command":"pwd","tool_call_id":"call1"}]}
        }});
        let read = MiniSweSessionFormat.read_line(&response.to_string());
        assert_eq!(read.content.role, Some(Role::Assistant));
        assert!(!read.content.ends_turn);
        let u = read.content.usage.expect("response usage");
        assert_eq!(
            (u.input_tokens, u.cache_read_tokens, u.output_tokens),
            (20, 100, 20)
        );
        assert!(matches!(&read.content.blocks[1],Block::ToolUse{id,..} if id=="call1"));
        let exit = json!({"type":"message","message":{"role":"exit","content":"DONE","extra":{"exit_status":"Submitted"}}});
        assert!(
            MiniSweSessionFormat
                .read_line(&exit.to_string())
                .content
                .ends_turn
        );
    }

    #[test]
    fn retargeting_preserves_native_reasoning_and_call_identifiers() {
        let raw = r#"{"type":"message","id":"native","message":{"object":"response","output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","call_id":"original"}]}}"#;
        assert_eq!(MiniSweSessionFormat.retarget(raw, "fork"), raw);
        let header = MiniSweSessionFormat
            .retarget(r#"{"type":"session","id":"source","version":1}"#, "fork");
        assert_eq!(
            serde_json::from_str::<Value>(&header).expect("header")["id"],
            "fork"
        );
    }
}

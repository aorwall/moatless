//! The ACP harness's own session file: one JSONL journal per Moatless session,
//! written from the agent's `session/update` stream.
//!
//! A `binding` line names the agent session that continues the journal. A
//! fork retargets the journal to a new id, which detaches every binding it
//! carries: the agent's session holds the parent's later turns too, so the
//! fork reseeds a fresh session from the journal instead.
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::super::content::CanonicalContentBlock as Block;
use super::super::source::Role;
use super::super::trajectory::{EventContent, RecordedLine, RenderContext, SessionFormat, Usage};

/// The longest tool result a reseed transcript quotes, in characters.
const SEED_RESULT_CHARS: usize = 2_000;

pub(crate) struct AcpSessionFormat;

pub(super) fn journal_path(home: &Path, id: &str) -> PathBuf {
    home.join("sessions").join(format!("{id}.jsonl"))
}

fn blocks(line: &Value) -> Vec<Block> {
    let text = || line["text"].as_str().unwrap_or_default().to_string();
    match line["type"].as_str() {
        Some("user" | "assistant") => vec![Block::Text { text: text() }],
        Some("thought") => vec![Block::Thinking {
            thinking: text(),
            signature: String::new(),
        }],
        Some("tool_call") => vec![Block::ToolUse {
            id: line["tool_call_id"].as_str().unwrap_or_default().into(),
            name: line["name"].as_str().unwrap_or_default().into(),
            input: line["input"].clone(),
        }],
        Some("tool_result") => vec![Block::ToolResult {
            tool_use_id: line["tool_call_id"].as_str().unwrap_or_default().into(),
            content: Some(line["content"].clone()),
            is_error: line["is_error"].as_bool(),
        }],
        _ => Vec::new(),
    }
}

fn usage(line: &Value) -> Option<Usage> {
    let u = line.get("usage")?;
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Some(Usage {
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
        cache_read_tokens: n("cache_read_tokens"),
        cache_write_tokens: n("cache_write_tokens"),
    })
}

impl SessionFormat for AcpSessionFormat {
    fn harness(&self) -> &'static str {
        "acp"
    }
    fn session_path(&self, home: &Path, _cwd: &Path, id: &str, _first: &str) -> PathBuf {
        journal_path(home, id)
    }
    fn locate(&self, home: &Path, _cwd: &Path, id: &str) -> Option<PathBuf> {
        let path = journal_path(home, id);
        path.is_file().then_some(path)
    }
    fn latest(&self, _home: &Path, _cwd: &Path) -> Option<String> {
        None
    }
    fn retarget(&self, payload: &str, id: &str) -> String {
        let Ok(mut v) = serde_json::from_str::<Value>(payload) else {
            return payload.into();
        };
        match v["type"].as_str() {
            Some("session") => v["id"] = json!(id),
            Some("binding") if v["journal"] != id => v["type"] = json!("detached_binding"),
            _ => return payload.into(),
        }
        v.to_string()
    }
    fn read_line(&self, line: &str) -> RecordedLine {
        let v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let kind = v["type"].as_str().unwrap_or("unparsed");
        let role = match kind {
            "user" | "tool_result" => Some(Role::User),
            "assistant" | "thought" | "tool_call" => Some(Role::Assistant),
            _ => None,
        };
        RecordedLine {
            kind: kind.into(),
            harness_item_id: v["id"].as_str().map(str::to_string),
            harness_turn_id: None,
            content: EventContent {
                role,
                blocks: blocks(&v),
                model: v["model"].as_str().map(str::to_string),
                usage: (kind == "turn_end").then(|| usage(&v)).flatten(),
                ends_turn: kind == "turn_end",
                api_error: kind == "turn_end" && v["failed"] == true,
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
            lines.push(header(ctx.harness_session_id).to_string());
        }
        for item in items {
            if item.injected || item.derived_from.is_some() {
                continue;
            }
            let assistant = item.role == Some(Role::Assistant);
            for block in &item.blocks {
                let id = uuid::Uuid::new_v4().to_string();
                let line = match block {
                    Block::Text { text } if assistant => {
                        json!({"type": "assistant", "id": id, "model": ctx.model, "text": text})
                    }
                    Block::Text { text } => json!({"type": "user", "id": id, "text": text}),
                    Block::Thinking { thinking, .. } => {
                        json!({"type": "thought", "id": id, "text": thinking})
                    }
                    Block::ToolUse {
                        id: call,
                        name,
                        input,
                    } => {
                        json!({"type": "tool_call", "id": id, "tool_call_id": call, "name": name, "input": input})
                    }
                    Block::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => json!({
                        "type": "tool_result", "id": id, "tool_call_id": tool_use_id,
                        "content": content, "is_error": is_error,
                    }),
                    _ => continue,
                };
                lines.push(line.to_string());
            }
            if item.ends_turn {
                lines.push(
                    json!({"type": "turn_end", "id": uuid::Uuid::new_v4().to_string()}).to_string(),
                );
            }
        }
        lines
    }
}

pub(super) fn header(id: &str) -> Value {
    json!({"type": "session", "version": 1, "id": id})
}

/// The agent session the journal's last binding for `agent` names.
pub(super) fn bound_session(journal: &str, agent: &str) -> Option<String> {
    journal
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["type"] == "binding" && v["agent"] == agent)
        .filter_map(|v| v["acp_session"].as_str().map(str::to_string))
        .next_back()
}

/// The journal as a transcript a fresh agent session reads before its first
/// prompt, or `None` when it holds no conversation yet.
pub(super) fn seed(journal: &str) -> Option<String> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for v in journal
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        let text = v["text"].as_str().unwrap_or_default();
        match v["type"].as_str() {
            Some("user") => {
                let _ = write!(out, "[user]\n{text}\n\n");
            }
            Some("assistant") => {
                let _ = write!(out, "[assistant]\n{text}\n\n");
            }
            Some("tool_call") => {
                let _ = writeln!(
                    out,
                    "[tool call {}] {}",
                    v["name"].as_str().unwrap_or_default(),
                    v["input"]
                );
            }
            Some("tool_result") => {
                let content = match &v["content"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let clipped: String = content.chars().take(SEED_RESULT_CHARS).collect();
                let more = if clipped.len() < content.len() {
                    " …"
                } else {
                    ""
                };
                let _ = write!(out, "[tool result]\n{clipped}{more}\n\n");
            }
            _ => {}
        }
    }
    (!out.is_empty()).then(|| {
        format!(
            "<conversation_history>\nThis conversation continues one held in an earlier session. \
             Its history so far:\n\n{out}</conversation_history>\n\n"
        )
    })
}

/// Appends lines to an open journal.
///
/// Writes are synchronous, deliberately: a line is a few KiB to the page
/// cache, and an await here would let the driver drop the harness's `next`
/// future between a line and the event it belongs to.
pub(super) struct Journal {
    file: std::fs::File,
    pub(super) id: String,
}

impl Journal {
    /// Opens the journal `id` under `home`, writing its header when new.
    pub(super) fn open(home: &Path, id: &str) -> std::io::Result<(Self, String)> {
        let path = journal_path(home, id);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let mut journal = Self {
            file,
            id: id.to_string(),
        };
        if existing.is_empty() {
            journal.append(&header(id));
        }
        Ok((journal, existing))
    }

    pub(super) fn append(&mut self, line: &Value) {
        let mut bytes = line.to_string().into_bytes();
        bytes.push(b'\n');
        if let Err(error) = self.file.write_all(&bytes) {
            tracing::warn!(%error, journal = %self.id, "Could not append to the ACP journal");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<String> {
        vec![
            header("j1").to_string(),
            json!({"type":"binding","agent":"antigravity","journal":"j1","acp_session":"s1"}).to_string(),
            json!({"type":"user","id":"u1","text":"Remember 42"}).to_string(),
            json!({"type":"tool_call","id":"c1","tool_call_id":"t1","name":"run_command","input":{"cmd":"ls"}}).to_string(),
            json!({"type":"tool_result","id":"r1","tool_call_id":"t1","content":"a.txt","is_error":false}).to_string(),
            json!({"type":"assistant","id":"a1","model":"gemini-3-flash","text":"Noted."}).to_string(),
            json!({"type":"turn_end","id":"e1","usage":{"input_tokens":10,"output_tokens":3,"cache_read_tokens":90,"cache_write_tokens":0}}).to_string(),
        ]
    }

    #[test]
    fn lines_keep_tool_ids_and_the_turn_boundary_carries_usage() {
        let read: Vec<_> = lines()
            .iter()
            .map(|l| AcpSessionFormat.read_line(l))
            .collect();
        assert!(matches!(&read[3].content.blocks[0], Block::ToolUse { id, .. } if id == "t1"));
        assert_eq!(read[4].content.role, Some(Role::User));
        assert!(
            matches!(&read[4].content.blocks[0], Block::ToolResult { tool_use_id, .. } if tool_use_id == "t1")
        );
        let end = &read[6].content;
        assert!(end.ends_turn && end.role.is_none());
        assert_eq!(end.usage.as_ref().map(|u| u.cache_read_tokens), Some(90));
    }

    #[test]
    fn a_fork_detaches_the_parents_agent_session_and_reseeds() {
        let joined = lines().join("\n");
        assert_eq!(bound_session(&joined, "antigravity").as_deref(), Some("s1"));
        let forked: Vec<_> = lines()
            .iter()
            .map(|l| AcpSessionFormat.retarget(l, "j2"))
            .collect();
        assert_eq!(bound_session(&forked.join("\n"), "antigravity"), None);
        let resumed: Vec<_> = lines()
            .iter()
            .map(|l| AcpSessionFormat.retarget(l, "j1"))
            .collect();
        assert_eq!(resumed, lines());
        let seed = seed(&forked.join("\n")).expect("conversation to carry");
        assert!(seed.contains("Remember 42") && seed.contains("a.txt") && seed.contains("Noted."));
    }

    #[test]
    fn another_harness_renders_into_lines_that_read_back() {
        let items = vec![
            EventContent {
                role: Some(Role::User),
                blocks: vec![Block::Text { text: "hi".into() }],
                ..EventContent::default()
            },
            EventContent {
                role: Some(Role::Assistant),
                blocks: vec![Block::Text {
                    text: "hello".into(),
                }],
                ends_turn: true,
                ..EventContent::default()
            },
        ];
        let ctx = RenderContext {
            harness_session_id: "j3",
            cwd: Path::new("/w"),
            model: Some("gemini-3-flash"),
            prior: &[],
            timestamp: "2026-11-21T00:00:00Z",
        };
        let rendered = AcpSessionFormat.render(&items, &ctx);
        let kinds: Vec<_> = rendered
            .iter()
            .map(|l| AcpSessionFormat.read_line(l).kind)
            .collect();
        assert_eq!(kinds, ["session", "user", "assistant", "turn_end"]);
        assert!(seed(&rendered.join("\n")).unwrap().contains("hello"));
    }
}

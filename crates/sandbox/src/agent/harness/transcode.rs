//! Rendering another harness's conversation as session lines of this one: the
//! prefix of a switch session (S4). Each format maps the other harness's tool
//! calls onto its own tools, and describes a call it has no tool for in text.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use super::content::CanonicalContentBlock;
use super::source::Role;
use super::trajectory::EventContent;

/// Where rendered lines go.
pub struct RenderContext<'a> {
    pub harness_session_id: &'a str,
    pub cwd: &'a Path,
    /// Stamped on rendered Claude assistant lines; Codex lines carry none.
    pub model: Option<&'a str>,
    /// The session's lines before the rendered ones; none renders a whole
    /// file, which Codex starts with its `session_meta` line.
    pub prior: &'a [&'a str],
    /// RFC 3339 time stamped on every rendered line.
    pub timestamp: &'a str,
}

/// Seeds the uuid a rendered Claude line gets from its session and index.
const LINE_NAMESPACE: uuid::Uuid = uuid::uuid!("6f1c7f0e-7d7b-4a53-9a39-6f4e3c1d2b10");
const CODEX_CLI_VERSION: &str = "0.156.1";
const CLAUDE_CLI_VERSION: &str = "2.1.281";

/// The blocks a transcode keeps: text, and tool calls paired with their
/// results. A call cut off before its result, or a result whose call precedes
/// the transcoded part, would leave either CLI with an unanswered call.
fn paired(items: &[EventContent]) -> Vec<(Role, Vec<&CanonicalContentBlock>)> {
    let blocks = || items.iter().flat_map(|item| item.blocks.iter());
    let calls: HashSet<&str> = blocks()
        .filter_map(|b| match b {
            CanonicalContentBlock::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let results: HashSet<&str> = blocks()
        .filter_map(|b| match b {
            CanonicalContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    items
        .iter()
        .filter_map(|item| {
            let role = match item.role? {
                Role::System => return None,
                role => role,
            };
            let kept = item
                .blocks
                .iter()
                .filter(|b| match b {
                    CanonicalContentBlock::Text { .. } => true,
                    CanonicalContentBlock::ToolUse { id, .. } => results.contains(id.as_str()),
                    CanonicalContentBlock::ToolResult { tool_use_id, .. } => {
                        calls.contains(tool_use_id.as_str())
                    }
                    _ => false,
                })
                .collect();
            Some((role, kept))
        })
        .collect()
}

fn results_by_call(
    items: &[(Role, Vec<&CanonicalContentBlock>)],
) -> HashMap<String, Option<Value>> {
    items
        .iter()
        .flat_map(|(_, blocks)| blocks.iter())
        .filter_map(|b| match b {
            CanonicalContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some((tool_use_id.clone(), content.clone())),
            _ => None,
        })
        .collect()
}

/// A tool result as the plain text both CLIs accept.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| p.is_object())
            .map(|p| p.get("text").and_then(Value::as_str).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => "null".to_string(),
    }
}

/// The text standing for a call this format has no tool for, and its result.
fn described(name: &str, input: &Value, result: Option<&Value>) -> String {
    format!(
        "[Tool call `{name}` with input {input}, made in this conversation by another agent \
         harness. It returned:\n{}]",
        result_text(result)
    )
}

fn prefixed(text: &str, sign: char) -> String {
    text.lines().fold(String::new(), |mut out, l| {
        let _ = writeln!(out, "{sign}{l}");
        out
    })
}

/// A Codex call for a Claude one: `(custom, name, arguments or input)`.
fn codex_call(name: &str, input: &Value) -> Option<(bool, &'static str, String)> {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    match name {
        "Bash" => Some((
            false,
            "exec_command",
            json!({ "cmd": field("command")? }).to_string(),
        )),
        "Write" => Some((
            true,
            "apply_patch",
            format!(
                "*** Begin Patch\n*** Add File: {}\n{}*** End Patch\n",
                field("file_path")?,
                prefixed(field("content")?, '+')
            ),
        )),
        "Edit" => Some((
            true,
            "apply_patch",
            format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n{}{}*** End Patch\n",
                field("file_path")?,
                prefixed(field("old_string")?, '-'),
                prefixed(field("new_string")?, '+')
            ),
        )),
        _ => None,
    }
}

/// A Claude `Bash` command for a Codex shell call.
fn claude_command(name: &str, input: &Value) -> Option<String> {
    if !matches!(name, "exec_command" | "shell" | "local_shell") {
        return None;
    }
    let cmd = input
        .get("cmd")
        .filter(|c| !c.is_null() && c.as_str() != Some(""))
        .or_else(|| input.get("command"))?;
    match cmd {
        Value::String(cmd) => Some(cmd.clone()),
        Value::Array(argv) => {
            let argv: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
            match argv.as_slice() {
                ["bash", "-lc", .., last] => Some((*last).to_string()),
                _ => Some(argv.join(" ")),
            }
        }
        _ => None,
    }
}

/// `items` as Codex rollout lines.
pub(super) fn render_codex(items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String> {
    let items = paired(items);
    let results = results_by_call(&items);
    let mut out = Vec::new();
    if ctx.prior.is_empty() {
        out.push(json!({
            "timestamp": ctx.timestamp,
            "type": "session_meta",
            "payload": {
                "id": ctx.harness_session_id,
                "timestamp": ctx.timestamp,
                "cwd": ctx.cwd,
                "originator": "moatless_transcode",
                "cli_version": CODEX_CLI_VERSION,
                "source": "exec",
            },
        }));
    }
    let item = |payload: Value| json!({"timestamp": ctx.timestamp, "type": "response_item", "payload": payload});
    let message = |role: &str, kind: &str, text: &str| {
        item(json!({"type": "message", "role": role, "content": [{"type": kind, "text": text}]}))
    };
    let (mut described_calls, mut custom_calls) = (HashSet::new(), HashSet::new());
    for (role, blocks) in &items {
        for block in blocks {
            match block {
                CanonicalContentBlock::Text { text } => out.push(match role {
                    Role::User => message("user", "input_text", text),
                    _ => message("assistant", "output_text", text),
                }),
                CanonicalContentBlock::ToolUse { id, name, input } => {
                    match codex_call(name, input) {
                        None => {
                            described_calls.insert(id.as_str());
                            let text = described(name, input, results[id].as_ref());
                            out.push(message("assistant", "output_text", &text));
                        }
                        Some((false, name, arguments)) => out.push(item(json!({
                            "type": "function_call", "name": name, "arguments": arguments, "call_id": id,
                        }))),
                        Some((true, name, input)) => {
                            custom_calls.insert(id.as_str());
                            out.push(item(json!({
                                "type": "custom_tool_call", "status": "completed", "call_id": id,
                                "name": name, "input": input,
                            })));
                        }
                    }
                }
                CanonicalContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if !described_calls.contains(tool_use_id.as_str()) => {
                    let kind = if custom_calls.contains(tool_use_id.as_str()) {
                        "custom_tool_call_output"
                    } else {
                        "function_call_output"
                    };
                    out.push(item(json!({
                        "type": kind, "call_id": tool_use_id, "output": result_text(content.as_ref()),
                    })));
                }
                _ => {}
            }
        }
    }
    out.iter().map(Value::to_string).collect()
}

/// `items` as Claude transcript lines, chained by `parentUuid` onto the last
/// prior line with a `uuid`: Claude walks the chain back from its leaf, so a
/// parent on a line without one (`attachment`, `last-prompt`) hides the prior
/// lines.
pub(super) fn render_claude(items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String> {
    let items = paired(items);
    let results = results_by_call(&items);
    let mut parent: Option<String> = ctx.prior.iter().rev().find_map(|line| {
        let line = serde_json::from_str::<Value>(line).ok()?;
        line.get("uuid").and_then(Value::as_str).map(str::to_string)
    });
    let mut out = Vec::new();
    let mut line = |role: Role, content: Value| {
        let uuid = uuid::Uuid::new_v5(
            &LINE_NAMESPACE,
            format!("{}:{}", ctx.harness_session_id, out.len()).as_bytes(),
        );
        let message = match role {
            Role::User => json!({"role": "user", "content": content}),
            _ => json!({
                "id": format!("msg_{}", uuid.simple()), "type": "message", "role": "assistant",
                "model": ctx.model.unwrap_or("<synthetic>"), "content": content,
                "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0},
            }),
        };
        out.push(json!({
            "parentUuid": parent, "isSidechain": false,
            "type": if role == Role::User { "user" } else { "assistant" },
            "uuid": uuid.to_string(), "timestamp": ctx.timestamp, "userType": "external",
            "cwd": ctx.cwd, "sessionId": ctx.harness_session_id, "version": CLAUDE_CLI_VERSION,
            "message": message,
        }));
        parent = Some(uuid.to_string());
    };
    let mut described_calls = HashSet::new();
    for (role, blocks) in &items {
        for block in blocks {
            match block {
                CanonicalContentBlock::Text { text } if *role == Role::User => {
                    line(Role::User, json!(text));
                }
                CanonicalContentBlock::Text { text } => {
                    line(Role::Assistant, json!([{"type": "text", "text": text}]));
                }
                CanonicalContentBlock::ToolUse { id, name, input } => {
                    let content = if let Some(command) = claude_command(name, input) {
                        json!([{
                            "type": "tool_use", "id": id, "name": "Bash",
                            "input": {"command": command},
                        }])
                    } else {
                        described_calls.insert(id.as_str());
                        let text = described(name, input, results[id].as_ref());
                        json!([{"type": "text", "text": text}])
                    };
                    line(Role::Assistant, content);
                }
                CanonicalContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if !described_calls.contains(tool_use_id.as_str()) => {
                    line(
                        Role::User,
                        json!([{
                            "type": "tool_result", "tool_use_id": tool_use_id,
                            "content": result_text(content.as_ref()),
                        }]),
                    );
                }
                _ => {}
            }
        }
    }
    out.iter().map(Value::to_string).collect()
}

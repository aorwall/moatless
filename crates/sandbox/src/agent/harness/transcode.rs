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
        .filter(|item| item.derived_from.is_none())
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

/// `text` single-quoted for a POSIX shell.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn hunk(old: &str, new: &str) -> String {
    format!("@@\n{}{}", prefixed(old, '-'), prefixed(new, '+'))
}

/// The shell command standing for a Claude `Read`, `Grep` or `Glob`.
fn claude_search(name: &str, input: &Value) -> Option<String> {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let number = |key: &str| input.get(key).and_then(Value::as_u64);
    let path = || field("path").map_or_default(|p| format!(" {}", quoted(p)));
    match name {
        "Read" => {
            let file = quoted(field("file_path")?);
            Some(match (number("offset"), number("limit")) {
                (None, None) => format!("cat -n {file}"),
                (offset, limit) => {
                    let first = offset.unwrap_or(1).max(1);
                    let last = limit.map_or("$".to_string(), |l| (first + l - 1).to_string());
                    format!("cat -n {file} | sed -n '{first},{last}p'")
                }
            })
        }
        "Grep" => {
            let mut cmd = String::from("rg");
            cmd += match field("output_mode") {
                Some("content") => " -n",
                Some("count") => " -c",
                _ => " -l",
            };
            if input.get("-i").and_then(Value::as_bool) == Some(true) {
                cmd += " -i";
            }
            if input.get("multiline").and_then(Value::as_bool) == Some(true) {
                cmd += " -U --multiline-dotall";
            }
            for flag in ["-A", "-B", "-C"] {
                if let Some(n) = number(flag) {
                    let _ = write!(cmd, " {flag} {n}");
                }
            }
            if let Some(glob) = field("glob") {
                let _ = write!(cmd, " -g {}", quoted(glob));
            }
            if let Some(kind) = field("type") {
                let _ = write!(cmd, " -t {}", quoted(kind));
            }
            let _ = write!(cmd, " -e {}{}", quoted(field("pattern")?), path());
            if let Some(n) = number("head_limit") {
                let _ = write!(cmd, " | head -n {n}");
            }
            Some(cmd)
        }
        "Glob" => Some(format!(
            "rg --files -g {}{}",
            quoted(field("pattern")?),
            path()
        )),
        _ => None,
    }
}

/// A Codex call for a Claude one: `(custom, name, arguments or input)`.
fn codex_call(name: &str, input: &Value) -> Option<(bool, &'static str, String)> {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let patch = |op: &str, body: String| {
        Some((
            true,
            "apply_patch",
            format!(
                "*** Begin Patch\n*** {op}: {}\n{body}*** End Patch\n",
                field("file_path")?
            ),
        ))
    };
    match name {
        "Bash" => Some((
            false,
            "exec_command",
            json!({ "cmd": field("command")? }).to_string(),
        )),
        "Read" | "Grep" | "Glob" => Some((
            false,
            "exec_command",
            json!({ "cmd": claude_search(name, input)? }).to_string(),
        )),
        "Write" => patch("Add File", prefixed(field("content")?, '+')),
        "Edit" => patch(
            "Update File",
            hunk(field("old_string")?, field("new_string")?),
        ),
        "TodoWrite" => {
            let todos = input.get("todos")?.as_array()?;
            let plan = todos.iter().map(|todo| {
                let text = |key: &str| todo.get(key).and_then(Value::as_str);
                Some(json!({"step": text("content")?, "status": text("status")?}))
            });
            let plan = plan.collect::<Option<Vec<_>>>()?;
            Some((false, "update_plan", json!({ "plan": plan }).to_string()))
        }
        _ => None,
    }
}

/// The command of a Codex shell call.
fn codex_command(input: &Value) -> Option<String> {
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

/// The argument object of a code-mode `exec` script that makes one
/// `tools.exec_command` call with JSON values; `None` for any other script.
fn code_mode_call(script: &str) -> Option<Value> {
    if script.matches("tools.").count() != 1 {
        return None;
    }
    let (_, call) = script.split_once("tools.exec_command(")?;
    let mut rest = call.trim_start().strip_prefix('{')?;
    let mut args = serde_json::Map::new();
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix('}') {
            return after
                .trim_start()
                .starts_with(')')
                .then_some(Value::Object(args));
        }
        let (key, after) = match rest.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"')?,
            None => rest.split_at(rest.find(|c: char| !c.is_alphanumeric() && c != '_')?),
        };
        let after = after.trim_start().strip_prefix(':')?;
        let mut values = serde_json::Deserializer::from_str(after).into_iter::<Value>();
        let value = values.next()?.ok()?;
        args.insert(key.to_string(), value);
        rest = after[values.byte_offset()..].trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
}

/// A Claude `Write` or `Edit` input for a Codex patch that adds one file, or
/// changes one with a single hunk; `None` for any other patch.
fn patch_edit(patch: &str, cwd: &Path) -> Option<(&'static str, Value)> {
    let mut lines = patch
        .strip_prefix("*** Begin Patch\n")?
        .trim_end()
        .strip_suffix("*** End Patch")?
        .lines();
    let header = lines.next()?;
    let body: Vec<&str> = lines.filter(|l| *l != "*** End of File").collect();
    let file = |path: &str| cwd.join(path).to_string_lossy().into_owned();
    if let Some(path) = header.strip_prefix("*** Add File: ") {
        let content = body.iter().try_fold(String::new(), |mut out, l| {
            let _ = writeln!(out, "{}", l.strip_prefix('+')?);
            Some(out)
        })?;
        return Some((
            "Write",
            json!({"file_path": file(path), "content": content}),
        ));
    }
    let path = header.strip_prefix("*** Update File: ")?;
    let (first, rest) = body.split_first()?;
    if !first.starts_with("@@")
        || rest
            .iter()
            .any(|l| l.starts_with("@@") || l.starts_with("***"))
    {
        return None;
    }
    let (mut old, mut new) = (Vec::new(), Vec::new());
    for line in rest {
        let mut chars = line.chars();
        match (chars.next(), chars.as_str()) {
            (None | Some(' '), text) => {
                old.push(text);
                new.push(text);
            }
            (Some('-'), text) => old.push(text),
            (Some('+'), text) => new.push(text),
            _ => return None,
        }
    }
    Some((
        "Edit",
        json!({"file_path": file(path), "old_string": old.join("\n"), "new_string": new.join("\n")}),
    ))
}

/// A Claude call for a Codex one: `(name, input)`.
fn claude_call(name: &str, input: &Value, cwd: &Path) -> Option<(&'static str, Value)> {
    let call;
    let input = match name {
        "exec_command" | "shell" | "local_shell" => input,
        "exec" => {
            call = code_mode_call(input.as_str()?)?;
            &call
        }
        "apply_patch" => {
            let patch = input.as_str().or_else(|| input.get("input")?.as_str())?;
            return patch_edit(patch, cwd);
        }
        _ => return None,
    };
    let mut command = codex_command(input)?;
    let workdir = input.get("workdir").and_then(Value::as_str).map(Path::new);
    if let Some(dir) = workdir.filter(|dir| *dir != cwd) {
        command = format!("cd {} && {command}", quoted(&dir.to_string_lossy()));
    }
    Some(("Bash", json!({ "command": command })))
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
    // Codex 0.156 refuses to resume a file whose last line lacks the `ordinal`
    // its earlier lines carry, so rendered lines continue the prior's count.
    if let Some(first) = next_ordinal(ctx.prior) {
        for (line, ordinal) in out.iter_mut().zip(first..) {
            line["ordinal"] = json!(ordinal);
        }
    }
    out.iter().map(Value::to_string).collect()
}

/// The `ordinal` after the prior's last line; `None` when that line has none.
fn next_ordinal(prior: &[&str]) -> Option<u64> {
    let last: Value = serde_json::from_str(prior.last()?).ok()?;
    Some(last.get("ordinal")?.as_u64()? + 1)
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
                    let content = if let Some((name, input)) = claude_call(name, input, ctx.cwd) {
                        json!([{"type": "tool_use", "id": id, "name": name, "input": input}])
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

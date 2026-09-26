//! Harness session files as trajectory events: [`SessionFormat`] reads one
//! line and writes one back, [`write_session`] writes a whole session file out
//! of the backend's trajectory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use sandbox_proto::{ReadTrajectoryRequest, read_trajectory_request};
use tracing::{info, warn};

use super::content::CanonicalContentBlock;
use super::source::Role;
pub use super::transcode::RenderContext;
use super::transcode::{render_claude, render_codex};
use crate::backend_client::BackendClient;

/// One harness's session file format: read a line, write a session.
pub trait SessionFormat: Send + Sync {
    /// The `harness` value the backend stores for this format.
    fn harness(&self) -> &'static str;

    /// Where the harness looks for this session's file. `first_line` is the
    /// file's line 0, which dates a Codex rollout; Claude ignores it.
    fn session_path(
        &self,
        home: &Path,
        cwd: &Path,
        harness_session_id: &str,
        first_line: &str,
    ) -> PathBuf;

    /// The session's file if one exists on disk. Blocks on the filesystem;
    /// async code calls [`locate_session`] instead.
    fn locate(&self, home: &Path, cwd: &Path, harness_session_id: &str) -> Option<PathBuf>;

    /// Classify one session line and derive its content. Never fails: a line
    /// that is not JSON is `kind = "unparsed"` with empty content.
    fn read_line(&self, line: &str) -> RecordedLine;

    /// Rewrite the session-identifier fields in `payload` to
    /// `harness_session_id`, leaving every other byte as it was.
    fn retarget(&self, payload: &str, harness_session_id: &str) -> String;

    /// The session the harness's own resume-latest picks, or `None` when it
    /// has none on disk. Blocks on the filesystem.
    fn latest(&self, home: &Path, cwd: &Path) -> Option<String>;

    /// The subagent session files beside `session_file` whose spawning tool
    /// call is known. Blocks on the filesystem.
    fn subagents(&self, _session_file: &Path) -> Vec<SubagentFile> {
        Vec::new()
    }

    /// Renders another harness's items as lines of this format, in order.
    fn render(&self, items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String>;
}

/// A subagent's own session file and the parent's tool call that spawned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentFile {
    pub harness_session_id: String,
    pub path: PathBuf,
    pub tool_use_id: String,
}

/// One session line, classified.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedLine {
    pub kind: String,
    pub harness_item_id: Option<String>,
    pub harness_turn_id: Option<String>,
    pub content: EventContent,
}

/// A line's harness-neutral content; bookkeeping lines have no role and no
/// blocks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventContent {
    #[serde(
        serialize_with = "serialize_role",
        deserialize_with = "deserialize_role"
    )]
    pub role: Option<Role>,
    #[serde(deserialize_with = "deserialize_blocks")]
    pub blocks: Vec<CanonicalContentBlock>,
    /// The harness wrote this line itself (environment, instructions, command
    /// and hook output): never shown and never transcoded.
    pub injected: bool,
    /// Set on the first line of a model response; the tail clears it on the
    /// response's later lines, since Claude repeats it on each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// A compaction line; its summary text is in `blocks`.
    pub compaction: bool,
}

/// One model response's token counts. `input_tokens` excludes cached input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    fn read(value: &Value, input: &str, cache_read: &str, cache_write: &str) -> Option<Self> {
        let n = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        value.is_object().then(|| Self {
            input_tokens: n(input),
            output_tokens: n("output_tokens"),
            cache_read_tokens: n(cache_read),
            cache_write_tokens: n(cache_write),
        })
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "`serialize_with` passes the field by reference"
)]
fn serialize_role<S: serde::Serializer>(role: &Option<Role>, s: S) -> Result<S::Ok, S::Error> {
    match role {
        Some(Role::User) => s.serialize_str("user"),
        Some(Role::Assistant) => s.serialize_str("assistant"),
        Some(Role::System) => s.serialize_str("system"),
        None => s.serialize_none(),
    }
}

fn deserialize_role<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Role>, D::Error> {
    Ok(role_of(Option::<String>::deserialize(d)?.as_deref()))
}

/// Blocks as [`canonical_block`] reads them, so one this build does not know
/// is `Unknown` rather than a failed item.
fn deserialize_blocks<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<CanonicalContentBlock>, D::Error> {
    Ok(Vec::<Value>::deserialize(d)?
        .iter()
        .map(canonical_block)
        .collect())
}

impl EventContent {
    /// The `content_json` wire form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"role":null,"blocks":[],"injected":false,"compaction":false}"#.to_string()
        })
    }
}

/// Whether a user text block is one the harness wrote itself.
fn injected_text(blocks: &[CanonicalContentBlock], prefixes: &[&str]) -> bool {
    blocks.iter().any(|block| match block {
        CanonicalContentBlock::Text { text } => {
            let text = text.trim_start();
            prefixes.iter().any(|p| text.starts_with(p))
        }
        _ => false,
    })
}

const CLAUDE_INJECTED: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<local-command-stdout>",
    "<local-command-caveat>",
    "<system-reminder>",
];

const CODEX_INJECTED: &[&str] = &[
    "<environment_context>",
    "<user_instructions>",
    "<skills_instructions>",
    "# AGENTS.md",
    "<turn_aborted>",
];

fn unparsed() -> RecordedLine {
    RecordedLine {
        kind: "unparsed".to_string(),
        harness_item_id: None,
        harness_turn_id: None,
        content: EventContent::default(),
    }
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |v, key| v.get(key))
        .and_then(Value::as_str)
}

fn role_of(role: Option<&str>) -> Option<Role> {
    match role? {
        "user" => Some(Role::User),
        "assistant" => Some(Role::Assistant),
        "system" | "developer" => Some(Role::System),
        _ => None,
    }
}

/// A native block as the canonical type, or `Unknown` carrying it verbatim.
fn canonical_block(block: &Value) -> CanonicalContentBlock {
    serde_json::from_value(block.clone())
        .unwrap_or_else(|_| CanonicalContentBlock::Unknown { raw: block.clone() })
}

/// Replace `"key":"old"` with `"key":"new"` for every pair, textually, so the
/// rest of the line keeps its bytes and key order.
fn replace_fields(payload: &str, fields: &[(&str, String)], new_id: &str) -> String {
    let mut out = payload.to_string();
    for (key, old) in fields {
        if old == new_id {
            continue;
        }
        for sep in [":", ": "] {
            let from = format!("\"{key}\"{sep}\"{old}\"");
            let to = format!("\"{key}\"{sep}\"{new_id}\"");
            out = out.replace(&from, &to);
        }
    }
    out
}

/// Claude Code's `<config>/projects/<dashed-cwd>/<id>.jsonl` transcript.
pub struct ClaudeSessionFormat;

impl ClaudeSessionFormat {
    /// Claude's project directory name: every character of the canonical cwd
    /// that is not ASCII alphanumeric becomes `-`.
    pub fn project_dir(cwd: &Path) -> String {
        let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        cwd.to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }
}

impl SessionFormat for ClaudeSessionFormat {
    fn harness(&self) -> &'static str {
        "claude-code"
    }

    fn session_path(&self, home: &Path, cwd: &Path, id: &str, _first_line: &str) -> PathBuf {
        home.join("projects")
            .join(Self::project_dir(cwd))
            .join(format!("{id}.jsonl"))
    }

    fn locate(&self, home: &Path, cwd: &Path, id: &str) -> Option<PathBuf> {
        let path = self.session_path(home, cwd, id, "");
        path.is_file().then_some(path)
    }

    /// `--continue`'s pick: the most recently modified transcript.
    fn latest(&self, home: &Path, cwd: &Path) -> Option<String> {
        let dir = home.join("projects").join(Self::project_dir(cwd));
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let id = path
                    .extension()
                    .is_some_and(|e| e == "jsonl")
                    .then(|| path.file_stem()?.to_str().map(str::to_string))??;
                Some((entry.metadata().ok()?.modified().ok()?, id))
            })
            .max()
            .map(|(_, id)| id)
    }

    fn read_line(&self, line: &str) -> RecordedLine {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return unparsed();
        };
        let kind = str_at(&value, &["type"]).unwrap_or("unknown").to_string();
        let harness_item_id = str_at(&value, &["message", "id"])
            .or_else(|| str_at(&value, &["uuid"]))
            .map(str::to_string);
        let content = match kind.as_str() {
            "user" | "assistant" => claude_content(&value),
            _ => EventContent::default(),
        };
        RecordedLine {
            kind,
            harness_item_id,
            harness_turn_id: None,
            content,
        }
    }

    fn retarget(&self, payload: &str, id: &str) -> String {
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return payload.to_string();
        };
        let fields: Vec<(&str, String)> = str_at(&value, &["sessionId"])
            .map(|old| vec![("sessionId", old.to_string())])
            .unwrap_or_default();
        replace_fields(payload, &fields, id)
    }

    /// `<id>/subagents/agent-<agent>.jsonl`, named by the spawning call in the
    /// `agent-<agent>.meta.json` beside it; a file whose meta is not written
    /// yet is left for a later scan.
    fn subagents(&self, session_file: &Path) -> Vec<SubagentFile> {
        let Some(parent) = session_file.file_stem().and_then(|s| s.to_str()) else {
            return Vec::new();
        };
        let dir = session_file.with_extension("").join("subagents");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let agent = path
                    .file_name()?
                    .to_str()?
                    .strip_prefix("agent-")?
                    .strip_suffix(".jsonl")?
                    .to_string();
                let meta = std::fs::read(dir.join(format!("agent-{agent}.meta.json"))).ok()?;
                let meta: Value = serde_json::from_slice(&meta).ok()?;
                Some(SubagentFile {
                    harness_session_id: format!("{parent}:agent-{agent}"),
                    tool_use_id: str_at(&meta, &["toolUseId"])?.to_string(),
                    path,
                })
            })
            .collect()
    }

    fn render(&self, items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String> {
        render_claude(items, ctx)
    }
}

fn claude_content(value: &Value) -> EventContent {
    let Some(message) = value.get("message") else {
        return EventContent::default();
    };
    let blocks: Vec<CanonicalContentBlock> = match message.get("content") {
        Some(Value::String(text)) => vec![CanonicalContentBlock::Text { text: text.clone() }],
        Some(Value::Array(blocks)) => blocks.iter().map(canonical_block).collect(),
        _ => Vec::new(),
    };
    let role = role_of(str_at(message, &["role"]));
    let flag = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
    let compaction = flag("isCompactSummary");
    EventContent {
        injected: !compaction
            && (flag("isMeta")
                || (role == Some(Role::User) && injected_text(&blocks, CLAUDE_INJECTED))),
        usage: (role == Some(Role::Assistant))
            .then(|| message.get("usage"))
            .flatten()
            .and_then(|u| {
                Usage::read(
                    u,
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                )
            }),
        compaction,
        role,
        blocks,
    }
}

/// Codex's `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl` rollout.
pub struct CodexSessionFormat;

impl SessionFormat for CodexSessionFormat {
    fn harness(&self) -> &'static str {
        "codex"
    }

    fn session_path(&self, home: &Path, _cwd: &Path, id: &str, first_line: &str) -> PathBuf {
        let stamp = serde_json::from_str::<Value>(first_line)
            .ok()
            // `payload.timestamp` is the thread's creation, which names the
            // file and is the `rollout_path` Codex's state db resumes from.
            .and_then(|v| {
                str_at(&v, &["payload", "timestamp"])
                    .or_else(|| str_at(&v, &["timestamp"]))
                    .map(str::to_string)
            })
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map_or_else(chrono::Utc::now, |ts| ts.with_timezone(&chrono::Utc));
        home.join("sessions")
            .join(stamp.format("%Y/%m/%d").to_string())
            .join(format!(
                "rollout-{}-{id}.jsonl",
                stamp.format("%Y-%m-%dT%H-%M-%S")
            ))
    }

    fn locate(&self, home: &Path, _cwd: &Path, id: &str) -> Option<PathBuf> {
        let suffix = format!("-{id}.jsonl");
        let mut stack = vec![home.join("sessions")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
                {
                    return Some(path);
                }
            }
        }
        None
    }

    fn read_line(&self, line: &str) -> RecordedLine {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return unparsed();
        };
        let top = str_at(&value, &["type"]).unwrap_or("unknown");
        let payload = value.get("payload").cloned().unwrap_or(Value::Null);
        let kind = match str_at(&payload, &["type"]) {
            Some(inner) => format!("{top}/{inner}"),
            None => top.to_string(),
        };
        let is_item = top == "response_item";
        let content = match kind.as_str() {
            _ if is_item => codex_item_content(&payload),
            "compacted" => EventContent {
                role: Some(Role::User),
                blocks: vec![CanonicalContentBlock::Text {
                    text: str_at(&payload, &["message"])
                        .unwrap_or_default()
                        .to_string(),
                }],
                compaction: true,
                ..EventContent::default()
            },
            "event_msg/token_count" => EventContent {
                usage: payload.pointer("/info/last_token_usage").and_then(|u| {
                    let usage = Usage::read(
                        u,
                        "input_tokens",
                        "cached_input_tokens",
                        "cache_write_input_tokens",
                    )?;
                    // Codex counts cached input inside `input_tokens`.
                    Some(Usage {
                        input_tokens: usage.input_tokens.saturating_sub(usage.cache_read_tokens),
                        ..usage
                    })
                }),
                ..EventContent::default()
            },
            _ => EventContent::default(),
        };
        RecordedLine {
            harness_item_id: is_item
                .then(|| str_at(&payload, &["id"]).map(str::to_string))
                .flatten(),
            harness_turn_id: str_at(&payload, &["turn_id"]).map(str::to_string),
            content,
            kind,
        }
    }

    fn retarget(&self, payload: &str, id: &str) -> String {
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return payload.to_string();
        };
        let mut fields = Vec::new();
        collect_thread_ids(&value, &mut fields);
        if str_at(&value, &["type"]) == Some("session_meta") {
            if let Some(old) = str_at(&value, &["payload", "id"]) {
                fields.push(("id", old.to_string()));
            }
        }
        replace_fields(payload, &fields, id)
    }

    /// Codex never resumes its latest session: `build_resume` starts it fresh.
    fn latest(&self, _home: &Path, _cwd: &Path) -> Option<String> {
        None
    }

    fn render(&self, items: &[EventContent], ctx: &RenderContext<'_>) -> Vec<String> {
        render_codex(items, ctx)
    }
}

fn collect_thread_ids(value: &Value, out: &mut Vec<(&'static str, String)>) {
    match value {
        Value::Object(map) => {
            for key in ["thread_id", "session_id"] {
                if let Some(Value::String(old)) = map.get(key) {
                    if !out.iter().any(|(k, o)| *k == key && o == old) {
                        out.push((key, old.clone()));
                    }
                }
            }
            map.values().for_each(|v| collect_thread_ids(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_thread_ids(v, out)),
        _ => {}
    }
}

fn codex_text_blocks(content: Option<&Value>) -> Vec<CanonicalContentBlock> {
    let Some(Value::Array(parts)) = content else {
        return Vec::new();
    };
    parts
        .iter()
        .map(|part| match str_at(part, &["type"]) {
            Some("input_text" | "output_text" | "text") => CanonicalContentBlock::Text {
                text: str_at(part, &["text"]).unwrap_or_default().to_string(),
            },
            _ => CanonicalContentBlock::Unknown { raw: part.clone() },
        })
        .collect()
}

/// A tool call's arguments: the parsed object when they are a JSON string,
/// else the raw value.
fn tool_input(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::String(text.clone())),
        Some(other) => other.clone(),
        None => Value::Object(Map::new()),
    }
}

fn codex_item_content(item: &Value) -> EventContent {
    let call_id = || {
        str_at(item, &["call_id"])
            .or_else(|| str_at(item, &["id"]))
            .unwrap_or_default()
            .to_string()
    };
    let assistant = |blocks| EventContent {
        role: Some(Role::Assistant),
        blocks,
        ..EventContent::default()
    };
    match str_at(item, &["type"]) {
        Some("message") => {
            let role = role_of(str_at(item, &["role"]));
            let blocks = codex_text_blocks(item.get("content"));
            EventContent {
                injected: role == Some(Role::System)
                    || (role == Some(Role::User) && injected_text(&blocks, CODEX_INJECTED)),
                role,
                blocks,
                ..EventContent::default()
            }
        }
        Some("function_call" | "custom_tool_call" | "local_shell_call") => {
            assistant(vec![CanonicalContentBlock::ToolUse {
                id: call_id(),
                name: str_at(item, &["name"]).unwrap_or("shell").to_string(),
                input: tool_input(
                    item.get("arguments")
                        .or_else(|| item.get("input"))
                        .or_else(|| item.get("action")),
                ),
            }])
        }
        Some("function_call_output" | "custom_tool_call_output") => EventContent {
            role: Some(Role::User),
            blocks: vec![CanonicalContentBlock::ToolResult {
                tool_use_id: call_id(),
                content: item.get("output").cloned(),
                is_error: None,
            }],
            ..EventContent::default()
        },
        Some("reasoning") => {
            let summary: Vec<&str> = item
                .get("summary")
                .and_then(Value::as_array)
                .map(|parts| parts.iter().filter_map(|p| str_at(p, &["text"])).collect())
                .unwrap_or_default();
            if summary.is_empty() {
                return assistant(Vec::new());
            }
            assistant(vec![CanonicalContentBlock::Thinking {
                thinking: summary.join("\n\n"),
                signature: String::new(),
            }])
        }
        _ => EventContent {
            blocks: vec![CanonicalContentBlock::Unknown { raw: item.clone() }],
            ..EventContent::default()
        },
    }
}

/// Which trajectory session [`write_session`] writes out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteTarget {
    /// Resume this session.
    Resume(String),
    /// The Task's fork session, which the backend creates on first read.
    Fork,
    /// The Task's switch session into this harness, which the backend creates
    /// on first read.
    Switch,
}

impl WriteTarget {
    /// Whether the backend may answer with a session whose prefix is rendered
    /// from the other harness.
    fn opens(&self) -> bool {
        matches!(self, Self::Fork | Self::Switch)
    }
}

/// A session file [`write_session`] left on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub harness_session_id: String,
    /// The lines the trajectory holds; a recorder resumes at this seq.
    pub line_count: i64,
    /// The first seq the CLI writes, past the rendered prefix. Set for an
    /// opened session; the recorder sends it until an append is accepted.
    pub own_from_seq: Option<i64>,
    /// False when an existing file was kept, which may hold other lines past
    /// a matching count.
    pub rewritten: bool,
}

/// Writes the session file for `target` out of the backend's trajectory,
/// rendering any other-harness items after its lines with `model`.
///
/// Keeps an existing file whose line at the last seq matches, which is the same
/// session at least as far along. `None`, logged, when the trajectory holds no
/// whole session; the caller then falls back to the transcripts.tar restore.
pub async fn write_session(
    backend: &dyn BackendClient,
    format: &Arc<dyn SessionFormat>,
    task_id: &str,
    home: &Path,
    cwd: &Path,
    target: WriteTarget,
    model: Option<&str>,
) -> Option<Written> {
    let read = read_session(backend, format, task_id, &target).await?;
    let mut body = read.body;
    let mut total = read.line_count;
    if !read.transcode.is_empty() {
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let prior: Vec<&str> = body.lines().collect();
        let rendered = format.render(
            &read.transcode,
            &RenderContext {
                harness_session_id: &read.id,
                cwd,
                model,
                prior: &prior,
                timestamp: &timestamp,
            },
        );
        total += rendered.len() as i64;
        for line in rendered {
            body.push_str(&line);
            body.push('\n');
        }
    }
    if body.is_empty() {
        warn!(
            ?target,
            "Trajectory read has nothing to write; using the archive"
        );
        return None;
    }
    let id = read.id;
    let mut written = Written {
        harness_session_id: id.clone(),
        line_count: read.line_count,
        own_from_seq: target.opens().then_some(total),
        rewritten: false,
    };
    let existing = locate_session(format, home, cwd, &id).await;
    if let Some(path) = &existing {
        let tip = body.lines().last().unwrap_or_default();
        if line_at(path, total - 1).await.as_deref() == Some(tip) {
            return Some(written);
        }
    }
    let path = existing.unwrap_or_else(|| {
        let first = body.lines().next().unwrap_or_default();
        format.session_path(home, cwd, &id, first)
    });
    if let Err(error) = write_atomically(&path, body.as_bytes()).await {
        warn!(%error, path = %path.display(), "Failed to write the session file; using the archive");
        return None;
    }
    info!(harness_session_id = %id, lines = read.line_count, rendered = total - read.line_count,
        path = %path.display(), "Wrote the session file from the trajectory");
    written.rewritten = true;
    Some(written)
}

/// Rewrites `path` in place with what the trajectory holds for
/// `harness_session_id`, keeping the file's inode so a harness appending to
/// it goes on appending after the trajectory's lines. Returns the line count.
pub(crate) async fn reseed_session(
    backend: &dyn BackendClient,
    format: &Arc<dyn SessionFormat>,
    task_id: &str,
    harness_session_id: &str,
    path: &Path,
) -> Option<i64> {
    let target = WriteTarget::Resume(harness_session_id.to_string());
    let read = read_session(backend, format, task_id, &target).await?;
    if read.line_count == 0 {
        return None;
    }
    if let Err(error) = tokio::fs::write(path, read.body.as_bytes()).await {
        warn!(%error, path = %path.display(), "Failed to reseed the session file");
        return None;
    }
    info!(%harness_session_id, lines = read.line_count, "Reseeded the session file from the trajectory");
    Some(read.line_count)
}

/// A session as [`read_session`] reads it.
struct SessionRead {
    id: String,
    /// The lines, retargeted to `id` and newline-ended.
    body: String,
    line_count: i64,
    transcode: Vec<EventContent>,
}

/// The session `target` names; `None` when the trajectory cannot give a whole
/// session.
async fn read_session(
    backend: &dyn BackendClient,
    format: &Arc<dyn SessionFormat>,
    task_id: &str,
    target: &WriteTarget,
) -> Option<SessionRead> {
    let request = ReadTrajectoryRequest {
        task_id: task_id.to_string(),
        harness: format.harness().to_string(),
        target: Some(match target {
            WriteTarget::Resume(id) => {
                read_trajectory_request::Target::HarnessSessionId(id.clone())
            }
            WriteTarget::Fork => read_trajectory_request::Target::Fork(true),
            WriteTarget::Switch => read_trajectory_request::Target::Switch(true),
        }),
    };
    let read = match backend.read_trajectory(request).await {
        Ok(read) => read,
        Err(status) => {
            info!(?target, code = ?status.code(), message = status.message(),
                "Not writing the session from the trajectory; using the archive");
            return None;
        }
    };
    if read.lines.len() as i64 != read.header.line_count {
        warn!(
            ?target,
            line_count = read.header.line_count,
            lines = read.lines.len(),
            "Trajectory read is incomplete; using the archive"
        );
        return None;
    }
    let transcode = read
        .transcode
        .iter()
        .map(|item| serde_json::from_str(&item.content_json))
        .collect::<Result<Vec<EventContent>, _>>();
    let transcode = match transcode {
        Ok(items) => items,
        Err(error) => {
            warn!(?target, %error, "Trajectory transcode item is not content; using the archive");
            return None;
        }
    };
    let id = read.header.harness_session_id;
    let mut body = String::new();
    for line in &read.lines {
        body.push_str(&format.retarget(line, &id));
        body.push('\n');
    }
    Some(SessionRead {
        id,
        body,
        line_count: read.header.line_count,
        transcode,
    })
}

/// [`SessionFormat::locate`] on the blocking pool, since Codex's walks every
/// rollout directory.
pub(crate) async fn locate_session(
    format: &Arc<dyn SessionFormat>,
    home: &Path,
    cwd: &Path,
    harness_session_id: &str,
) -> Option<PathBuf> {
    let format = Arc::clone(format);
    let (home, cwd) = (home.to_path_buf(), cwd.to_path_buf());
    let id = harness_session_id.to_string();
    tokio::task::spawn_blocking(move || format.locate(&home, &cwd, &id))
        .await
        .ok()
        .flatten()
}

/// [`SessionFormat::latest`] on the blocking pool.
pub(crate) async fn latest_session(
    format: &Arc<dyn SessionFormat>,
    home: &Path,
    cwd: &Path,
) -> Option<String> {
    let format = Arc::clone(format);
    let (home, cwd) = (home.to_path_buf(), cwd.to_path_buf());
    tokio::task::spawn_blocking(move || format.latest(&home, &cwd))
        .await
        .ok()
        .flatten()
}

/// The file's complete line `n`, without its newline.
async fn line_at(path: &Path, n: i64) -> Option<String> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let complete = &bytes[..=bytes.iter().rposition(|b| *b == b'\n')?];
    let line = complete
        .split(|b| *b == b'\n')
        .nth(usize::try_from(n).ok()?)?;
    Some(String::from_utf8_lossy(line).into_owned())
}

async fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
#[path = "trajectory_tests.rs"]
mod tests;

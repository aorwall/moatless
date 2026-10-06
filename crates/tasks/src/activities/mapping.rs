//! What one arriving content block becomes.
//!
//! A pure function from a block and its context to the row it changes. Both the
//! live writer and the migration go through it, so they cannot diverge.

pub use moatless_types::activity::{
    ActivityKind, ActivityStatus, PLAN_APPROVE, PLAN_QUESTION, PLAN_REJECT, PLAN_TOOL,
    QUESTION_TOOL, item_type, result_text,
};
use moatless_types::codex::CODEX_TOOL_TODO_LIST;
use moatless_types::sandbox::SandboxEventPayload;
use serde_json::{Map, Value, json};

use moatless_types::message::MessageType;

/// What the writer already knows about a block before it decides anything.
#[derive(Debug, Clone, Copy)]
pub struct BlockContext<'a> {
    pub task_id: &'a str,
    pub message_uuid: &'a str,
    /// The `Agent` tool-use id this Message's subagent transcript hangs under,
    /// or `None` for the Task's own conversation.
    pub parent_uuid: Option<&'a str>,
    /// The Message's type, which is what decides whether text is a Message at
    /// all. Both readers already drop `system`, `result` and `error` text; this
    /// is where storage stops writing what they throw away.
    pub message_type: MessageType,
    /// The block's native index within its Message, from source-event
    /// `blockIndex`.
    pub block_index: usize,
    /// What an activity's id is named from: the Task and the source row the
    /// block arrived on (4.9).
    pub source: &'a str,
    /// The block's index within that source row.
    pub source_block: usize,
}

/// The row-level change one block represents.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockRow {
    /// The block's text, spliced into the Message at `block_index`.
    Text(TextPart),
    /// The rows one block writes: a tool call or thinking, plus — for calls a
    /// client renders as a panel — the joined row, sharing the block's ordinal
    /// and suffixing its id.
    Activities(Vec<ActivityRow>),
    /// A tool result, which updates the call's row rather than writing one.
    ToolOutcome(ToolOutcome),
    /// Nothing this conversation stores.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPart {
    pub message_uuid: String,
    pub block_index: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRow {
    pub activity_id: String,
    pub task_id: String,
    /// The source block's index, or 0 for a row with no block. Read off the
    /// block rather than counted while mapping, so it stays stable for exactly
    /// as long as `activity_id` does.
    pub ordinal: i32,
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    /// The call a later result settles by; set only on the call, since
    /// `(task_id, tool_use_id)` is unique — a derived row joins through
    /// `requestId` in its payload instead.
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    pub is_error: bool,
}

/// The update a `tool_result` block makes to the call it answers. Carries no
/// `message_uuid`: it arrives on a later Message than its call, so it is found
/// by `(task_id, tool_use_id)` instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub task_id: String,
    pub tool_use_id: String,
    pub status: ActivityStatus,
    /// The result whole. What the Moatless frontend renders, and what
    /// [`detail`] is cut from.
    pub result: Value,
    /// The transport's structured view of the outcome (the SDK's
    /// `toolUseResult`). Attached by the caller after mapping, since it belongs
    /// to the Message, not the block; `None` when the transport reported none.
    pub typed_result: Option<Value>,
    pub is_error: bool,
}

/// Everything one Message's content array changes, in one value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationWrite {
    pub text: Vec<TextPart>,
    pub activities: Vec<ActivityRow>,
    pub outcomes: Vec<ToolOutcome>,
    /// The Message's reported token usage, admitted once per Message.
    pub usage: Option<AdmittedUsage>,
}

/// A Message's per-tier token usage, admitted once against its uuid. The
/// cached-prefix reads and cache-creation writes are separate tiers from
/// `input_tokens`, so cost stays derivable per Message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdmittedUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
}

impl ConversationWrite {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.activities.is_empty()
            && self.outcomes.is_empty()
            && self.usage.is_none()
    }
}

/// Map a content array, each block at its own index.
///
/// `context.block_index` is the first block's index, keeping identity stable.
pub fn map_content(context: BlockContext<'_>, content: &Value) -> ConversationWrite {
    let mut write = ConversationWrite::default();
    let Some(blocks) = content.as_array() else {
        // A Message written as a bare string rather than a block list. 153 rows
        // in today's history are shaped this way; every one holds `{}`.
        if let Some(text) = content.as_str().filter(|text| !text.is_empty()) {
            if is_conversation(context.message_type) {
                write.text.push(TextPart {
                    message_uuid: context.message_uuid.to_string(),
                    block_index: context.block_index,
                    text: text.to_string(),
                });
            }
        }
        return write;
    };
    for (offset, block) in blocks.iter().enumerate() {
        let context = BlockContext {
            block_index: context.block_index + offset,
            source_block: context.source_block + offset,
            ..context
        };
        match map_block(context, block) {
            BlockRow::Text(part) => write.text.push(part),
            BlockRow::Activities(activities) => write.activities.extend(activities),
            BlockRow::ToolOutcome(outcome) => write.outcomes.push(outcome),
            BlockRow::None => {}
        }
    }
    write
}

/// What one block becomes.
pub fn map_block(context: BlockContext<'_>, block: &Value) -> BlockRow {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => text_part(context, block),
        Some("tool_use") => tool_call(context, block),
        Some("tool_result") => tool_outcome(context, block),
        Some("thinking" | "redacted_thinking") => thinking(context, block),
        // Silently, as `projection.rs` already does: an unrecognised block is an
        // SDK feature this conversation does not render, not a fault.
        _ => BlockRow::None,
    }
}

/// A block's text, but only where what was said is a Message: narrowed to
/// `user` and `assistant`. `system` is dropped, and `result`/`error` become the
/// run's closing line, not something somebody said.
fn text_part(context: BlockContext<'_>, block: &Value) -> BlockRow {
    if !is_conversation(context.message_type) {
        return BlockRow::None;
    }
    match block.get("text").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => BlockRow::Text(TextPart {
            message_uuid: context.message_uuid.to_string(),
            block_index: context.block_index,
            text: text.to_string(),
        }),
        _ => BlockRow::None,
    }
}

fn is_conversation(message_type: MessageType) -> bool {
    matches!(message_type, MessageType::User | MessageType::Assistant)
}

fn tool_call(context: BlockContext<'_>, block: &Value) -> BlockRow {
    let Some(tool_use_id) = block.get("id").and_then(Value::as_str) else {
        // Unlike an unrecognised block type, a `tool_use` with no id is
        // malformed rather than unsupported — and dropping it loses the call
        // *and* the result that will later fail to find it.
        tracing::warn!(
            task_id = %context.task_id,
            message_uuid = %context.message_uuid,
            "tool_use block carries no id; dropping the call and its result"
        );
        return BlockRow::None;
    };

    let name = block
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Tool call");
    let status = ActivityStatus::InProgress;

    let mut payload = Map::new();
    payload.insert("title".into(), Value::String(name.to_string()));
    payload.insert("itemType".into(), Value::String(item_type(name).into()));
    payload.insert("status".into(), Value::String(status.wire().into()));
    payload.insert("data".into(), json!({ "toolCallId": tool_use_id }));
    payload.insert(
        "input".into(),
        block.get("input").cloned().unwrap_or_else(|| json!({})),
    );
    // The client reads the command out of the payload to render a terminal row,
    // and falls back to the detail text for everything else.
    if let Some(command) = block
        .get("input")
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
    {
        payload.insert("command".into(), Value::String(command.to_string()));
    }

    let call = ActivityRow {
        activity_id: activity_id(context),
        task_id: context.task_id.to_string(),
        ordinal: ordinal(context),
        message_uuid: Some(context.message_uuid.to_string()),
        parent_uuid: context.parent_uuid.map(str::to_string),
        kind: ActivityKind::ToolCall,
        status,
        tool_use_id: Some(tool_use_id.to_string()),
        summary: name.to_string(),
        payload: Value::Object(payload),
        is_error: false,
    };

    let input = block.get("input").unwrap_or(&Value::Null);
    let mut rows = vec![call];
    rows.extend(joined_row(context, name, tool_use_id, input));
    BlockRow::Activities(rows)
}

/// The named row a call is *also* stored as, where a client derives a panel
/// from it rather than a work-log card. Four calls have one; the call itself is
/// still stored unchanged, so this adds a row rather than replacing one.
fn joined_row(
    context: BlockContext<'_>,
    name: &str,
    tool_use_id: &str,
    input: &Value,
) -> Option<ActivityRow> {
    let (suffix, kind, summary, payload) = match name {
        QUESTION_TOOL => (
            "user-input",
            ActivityKind::UserInputRequested,
            "Question",
            json!({
                "requestId": tool_use_id,
                "questions": asked_questions(input),
            }),
        ),
        PLAN_TOOL => (
            "user-input",
            ActivityKind::UserInputRequested,
            "Proposed plan",
            json!({
                "requestId": tool_use_id,
                "questions": [plan_question()],
            }),
        ),
        TODO_TOOL | CODEX_TOOL_TODO_LIST => (
            "plan",
            ActivityKind::PlanUpdated,
            "Plan updated",
            json!({ "plan": plan_steps(name, input) }),
        ),
        _ => return None,
    };
    // A question with nothing to ask, or a plan with no steps, is a call this
    // panel cannot be derived from. The tool call itself is still stored.
    if payload
        .get("questions")
        .or_else(|| payload.get("plan"))
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return None;
    }
    Some(ActivityRow {
        activity_id: format!("{}:{suffix}", activity_id(context)),
        task_id: context.task_id.to_string(),
        // The call's own ordinal: a joined row happened at the block the call
        // is, not after it.
        ordinal: ordinal(context),
        message_uuid: Some(context.message_uuid.to_string()),
        parent_uuid: context.parent_uuid.map(str::to_string),
        kind,
        status: ActivityStatus::Completed,
        // The column names the one row a result settles, and `(task_id,
        // tool_use_id)` is unique on exactly that reading. A joined row is
        // derived from the call rather than being it, and no result ever lands
        // on it, so it takes the join through `requestId` in its payload —
        // which is what every client reads — and leaves the column empty.
        tool_use_id: None,
        summary: summary.to_string(),
        payload,
        is_error: false,
    })
}

/// The Claude CLI's plan tracker; [`CODEX_TOOL_TODO_LIST`] is what the Codex
/// transport's `turn/plan/updated` notification is normalised into.
const TODO_TOOL: &str = "TodoWrite";

/// The questions an `AskUserQuestion` call asked, filled to the shape the panel
/// parses. The id **is** the question text the CLI answers by — a synthetic
/// index would answer nothing.
fn asked_questions(input: &Value) -> Vec<Value> {
    input
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .enumerate()
                .filter_map(|(index, question)| asked_question(index, question))
                .collect()
        })
        .unwrap_or_default()
}

fn asked_question(index: usize, question: &Value) -> Option<Value> {
    let text = question
        .get("question")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())?;
    let options: Vec<Value> = question
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = option
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty())?;
                    let description = option
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or(label);
                    Some(json!({ "label": label, "description": description }))
                })
                .collect()
        })
        .unwrap_or_default();
    if options.is_empty() {
        return None;
    }
    let header = question
        .get("header")
        .and_then(Value::as_str)
        .filter(|header| !header.is_empty())
        .map_or_else(|| format!("Question {}", index + 1), str::to_string);
    Some(json!({
        "id": text,
        "header": header,
        "question": text,
        "multiSelect": question.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
        "options": options,
    }))
}

fn plan_question() -> Value {
    json!({
        "id": PLAN_QUESTION,
        "header": "Proposed plan",
        "question": PLAN_QUESTION,
        "multiSelect": false,
        "options": [
            { "label": PLAN_APPROVE, "description": "Let the agent carry out the plan as proposed." },
            { "label": PLAN_REJECT, "description": "Send the agent back to planning. Say what to change." },
        ],
    })
}

/// One plan tracker's steps, each with its own three-value status —
/// `TodoWrite`'s and `codex.todo_list`'s alike — kept unflattened because *in
/// progress* is the state a plan panel exists to show.
fn plan_steps(tool: &str, input: &Value) -> Vec<Value> {
    let entries = if tool == TODO_TOOL {
        input.get("todos")
    } else {
        input.get("items")
    };
    entries
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let step = entry
                        .get("content")
                        .or_else(|| entry.get("text"))
                        .and_then(Value::as_str)
                        .filter(|step| !step.is_empty())?;
                    Some(json!({ "step": step, "status": step_status(entry) }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A step's state in the client's vocabulary. `status` is read first, since it
/// is the only signal of a step in flight; an entry with no status falls back
/// to its completed boolean.
fn step_status(entry: &Value) -> &'static str {
    match entry.get("status").and_then(Value::as_str) {
        Some("completed") => return "completed",
        Some("in_progress" | "inProgress") => return "inProgress",
        Some("pending") => return "pending",
        _ => {}
    }
    if entry.get("completed").and_then(Value::as_bool) == Some(true) {
        "completed"
    } else {
        "pending"
    }
}

fn tool_outcome(context: BlockContext<'_>, block: &Value) -> BlockRow {
    let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
        tracing::warn!(
            task_id = %context.task_id,
            message_uuid = %context.message_uuid,
            "tool_result block names no tool_use_id; it can answer no call"
        );
        return BlockRow::None;
    };

    let is_error = block
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    BlockRow::ToolOutcome(ToolOutcome {
        task_id: context.task_id.to_string(),
        tool_use_id: tool_use_id.to_string(),
        status: if is_error {
            ActivityStatus::Failed
        } else {
            ActivityStatus::Completed
        },
        result: block.get("content").cloned().unwrap_or(Value::Null),
        typed_result: None,
        is_error,
    })
}

/// The label the work log shows for a stretch of thinking.
pub const THINKING_SUMMARY: &str = "Thinking";

/// How much of one thought is stored. The full text was 301 MB across a
/// deployment; a capped copy is what a reasoning row renders.
pub const THINKING_TEXT_MAX_BYTES: usize = 16 * 1024;

/// Thinking stores its label and the thought's first
/// [`THINKING_TEXT_MAX_BYTES`]; `truncated` marks a cut copy.
fn thinking(context: BlockContext<'_>, block: &Value) -> BlockRow {
    let mut payload = Map::new();
    payload.insert("summary".into(), json!(THINKING_SUMMARY));
    if let Some(text) = block
        .get("thinking")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        let mut end = text.len().min(THINKING_TEXT_MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        payload.insert("text".into(), json!(&text[..end]));
        if end < text.len() {
            payload.insert("truncated".into(), json!(true));
        }
    }
    BlockRow::Activities(vec![ActivityRow {
        activity_id: activity_id(context),
        task_id: context.task_id.to_string(),
        ordinal: ordinal(context),
        message_uuid: Some(context.message_uuid.to_string()),
        parent_uuid: context.parent_uuid.map(str::to_string),
        kind: ActivityKind::Thinking,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: THINKING_SUMMARY.to_string(),
        payload: Value::Object(payload),
        is_error: false,
    }])
}

/// The three severities a Sandbox lifecycle event carries into the work log.
///
/// Row `kind`, `status`, and `is_error` derive from this; on T3 it colours a
/// heading: `Warning` amber, `Error` red, `Status` plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Status,
    Warning,
    Error,
}

/// The label and severity a Sandbox `lifecycleEvent` renders as.
///
/// `None` for an unknown slug; the call site falls back to the humanized slug
/// so a new event still renders readable.
pub fn lifecycle_descriptor(lifecycle_event: &str) -> Option<(&'static str, Severity)> {
    use Severity::{Error, Status, Warning};
    Some(match lifecycle_event {
        "pod_ready" => ("Ready", Status),
        "pod_scheduled" => ("Scheduled", Status),
        "pod_deleted" => ("Deleted", Status),
        "pod_succeeded" | "sandbox_completed" => ("Completed", Status),
        "auto_stopped" => ("Auto-stopped", Status),
        "idle_reaped" => ("Stopped (idle)", Status),
        "startup_slow" => ("Slow startup", Warning),
        "scheduling_blocked" => ("Scheduling blocked", Warning),
        "pod_evicted" => ("Evicted", Warning),
        "pod_recovering" => ("Recovering", Warning),
        "node_reclaimed" => ("Node reclaimed", Warning),
        "container_restarted" => ("Restarted", Warning),
        "container_oom_killed" => ("Out of memory", Error),
        "container_crash_loop" => ("Crash looping", Error),
        "container_terminated" => ("Terminated", Error),
        "pod_failed" => ("Pod failed", Error),
        "fatal" => ("Fatal", Error),
        "runtime_infrastructure" => ("Infrastructure error", Error),
        "runtime_rejected" => ("Rejected", Error),
        "runtime_not_absent" => ("Cleanup stalled", Error),
        "intent_write" => ("Intent write failed", Error),
        "completion_commit" => ("Completion commit failed", Error),
        "edge_abandoned" => ("Operation abandoned", Error),
        _ => return None,
    })
}

/// The label and severity for one Sandbox event.
///
/// With no `lifecycleEvent`, the label is the event's own message — init steps
/// share one sub-type, so labelling by it would collapse them — and severity
/// falls back to the sub-type's error-ness.
fn describe(
    coarse_status: &str,
    coarse_is_error: bool,
    payload: &SandboxEventPayload,
) -> (String, Severity) {
    let fallback = if coarse_is_error {
        Severity::Error
    } else {
        Severity::Status
    };
    match payload
        .lifecycle_event
        .as_deref()
        .filter(|event| !event.is_empty())
    {
        Some(event) => match lifecycle_descriptor(event) {
            Some((label, severity)) => (label.to_string(), severity),
            None => (event.replace('_', " "), fallback),
        },
        None => match payload.message.trim() {
            "" => (capitalize(coarse_status), fallback),
            message => (message.to_string(), fallback),
        },
    }
}

/// The coarse status word as a label: `ready` → `Ready`, `not_created` → `Not
/// created`.
fn capitalize(status: &str) -> String {
    let spaced = status.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

/// A whole number of seconds as a compact duration (`37s`, `6m 4s`, `1h 20m`),
/// matching `formatDurationSeconds` in the frontend event log.
fn format_duration_seconds(total_seconds: i64) -> String {
    let seconds = total_seconds.max(0);
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let rem_seconds = seconds % 60;
    if minutes < 60 {
        return if rem_seconds > 0 {
            format!("{minutes}m {rem_seconds}s")
        } else {
            format!("{minutes}m")
        };
    }
    let hours = minutes / 60;
    let rem_minutes = minutes % 60;
    if rem_minutes > 0 {
        format!("{hours}h {rem_minutes}m")
    } else {
        format!("{hours}h")
    }
}

/// A Sandbox lifecycle change, which happened on no Message.
///
/// Identity is the event's own id, so redelivery is a no-op;
/// `coarse_status`/`coarse_is_error` apply only with no `lifecycleEvent`, and
/// detail lines follow `buildChips` order.
pub fn sandbox_activity(
    event_id: &str,
    task_id: &str,
    coarse_status: &str,
    coarse_is_error: bool,
    payload: &SandboxEventPayload,
) -> ActivityRow {
    let (summary, severity) = describe(coarse_status, coarse_is_error, payload);
    let is_error = severity == Severity::Error;

    let mut lines: Vec<String> = Vec::new();
    if !payload.message.is_empty() && payload.message != summary {
        lines.push(payload.message.clone());
    }
    if let Some(node) = &payload.node_name {
        lines.push(format!("node {node}"));
    }
    if let Some(cause) = &payload.disruption_reason {
        lines.push(format!("cause {cause}"));
    }
    if let Some(by) = &payload.initiator {
        lines.push(format!("by {by}"));
    }
    if let Some(exit) = payload.exit_code {
        lines.push(format!("exit {exit}"));
    }
    if let Some(reason) = &payload.terminated_reason {
        lines.push(format!("reason {reason}"));
    }
    if let Some(pending) = payload.pending_seconds {
        lines.push(format!("pending {}", format_duration_seconds(pending)));
    }

    let mut row_payload = Map::new();
    row_payload.insert("summary".into(), Value::String(summary.clone()));
    if !lines.is_empty() {
        row_payload.insert("detail".into(), Value::String(lines.join("\n")));
    }

    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        // A Sandbox activity is alone in its change, so slot 0 is its own.
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: match severity {
            Severity::Status => ActivityKind::SandboxStatus,
            Severity::Warning => ActivityKind::SandboxWarning,
            Severity::Error => ActivityKind::SandboxError,
        },
        status: if is_error {
            ActivityStatus::Failed
        } else {
            ActivityStatus::Completed
        },
        tool_use_id: None,
        summary,
        payload: Value::Object(row_payload),
        is_error,
    }
}

/// What a person's answer becomes, written on the carrier row that relays it to
/// the agent — its own change, since the call it answers was written on an
/// earlier Message whose slots are already taken.
pub fn user_input_resolved(
    task_id: &str,
    message_uuid: &str,
    tool_use_id: &str,
    answers: &Value,
) -> ActivityRow {
    ActivityRow {
        activity_id: format!("{message_uuid}:0:user-input-resolved"),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: Some(message_uuid.to_string()),
        parent_uuid: None,
        kind: ActivityKind::UserInputResolved,
        status: ActivityStatus::Completed,
        // Empty for the reason a joined row's is: the column names the row a
        // result settles, and this row is not that row. The answer settles the
        // call through the outcome written beside it.
        tool_use_id: None,
        summary: "Answered".to_string(),
        // `requestId` is the tool-use id on both rows, which is the whole join:
        // the client closes the panel this names.
        payload: json!({ "requestId": tool_use_id, "answers": answers }),
        is_error: false,
    }
}

/// What the harness said about its own context window.
///
/// `max_tokens` is absent when unreported, so the meter shows a count with no
/// percentage rather than a guess; identified by the event that carried it, so
/// redelivery writes nothing.
pub fn context_window_updated(
    event_id: &str,
    task_id: &str,
    used_tokens: i64,
    max_tokens: Option<i64>,
) -> ActivityRow {
    let mut payload = Map::new();
    payload.insert("usedTokens".into(), json!(used_tokens));
    if let Some(max_tokens) = max_tokens {
        payload.insert("maxTokens".into(), json!(max_tokens));
    }
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: ActivityKind::ContextWindowUpdated,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: "Context window updated".to_string(),
        payload: Value::Object(payload),
        is_error: false,
    }
}

/// The platform moving a Task to another agent, which happened on no Message.
pub fn agent_switched(event_id: &str, task_id: &str, summary: &str) -> ActivityRow {
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        ordinal: 0,
        message_uuid: None,
        parent_uuid: None,
        kind: ActivityKind::AgentSwitched,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: summary.to_string(),
        payload: json!({ "summary": summary }),
        is_error: false,
    }
}

/// `{source}:{source_block}`, the name [`crate::conversation::ids::activity`]
/// hashes.
fn activity_id(context: BlockContext<'_>) -> String {
    format!("{}:{}", context.source, context.source_block)
}

/// The block's index, as the column stores it. Saturates rather than wraps: a
/// Message with more blocks than an `i32` holds is impossible in practice.
fn ordinal(context: BlockContext<'_>) -> i32 {
    i32::try_from(context.block_index).unwrap_or(i32::MAX)
}

/// Tool output long enough to be a file rather than a preview. The T3 surface
/// re-sends its whole log on every conversation move, so an uncapped `Read` of
/// a large file would repeat it in every send.
pub const DETAIL_LIMIT: usize = 2_000;

/// What a settled call shows before it is expanded, cut to [`DETAIL_LIMIT`].
///
/// `None` with no visible output; the cut is marked, not silent, and lands on a
/// character boundary to avoid panicking mid-codepoint.
pub fn detail(result: &Value) -> Option<String> {
    let text = result_text(result)?;
    if text.len() <= DETAIL_LIMIT {
        return Some(text.to_string());
    }
    let mut end = DETAIL_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}\n… truncated", &text[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> BlockContext<'static> {
        BlockContext {
            task_id: "task-1",
            message_uuid: "msg-1",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 3,
            source: "msg-1",
            source_block: 3,
        }
    }

    /// The single row a block wrote, for the blocks that join none.
    fn only(row: BlockRow) -> ActivityRow {
        let BlockRow::Activities(mut rows) = row else {
            panic!("expected activity rows, got {row:?}");
        };
        assert_eq!(rows.len(), 1, "this block joins no second row");
        rows.remove(0)
    }

    /// The rows a block wrote, in the order it wrote them: the call first, then
    /// whatever is joined to it.
    fn rows(row: BlockRow) -> Vec<ActivityRow> {
        let BlockRow::Activities(rows) = row else {
            panic!("expected activity rows, got {row:?}");
        };
        rows
    }

    #[test]
    fn a_text_block_lands_on_the_message_it_arrived_on() {
        assert_eq!(
            map_block(context(), &json!({ "type": "text", "text": "hello" })),
            BlockRow::Text(TextPart {
                message_uuid: "msg-1".into(),
                block_index: 3,
                text: "hello".into(),
            })
        );
    }

    #[test]
    fn empty_text_says_nothing() {
        assert_eq!(
            map_block(context(), &json!({ "type": "text", "text": "" })),
            BlockRow::None
        );
    }

    /// Decision 13: only `user` and `assistant` text survives, because that is
    /// the exclusion both readers already apply on every read.
    #[test]
    fn text_on_a_row_nobody_said_is_not_stored() {
        for dropped in [MessageType::System, MessageType::Result, MessageType::Error] {
            let context = BlockContext {
                message_type: dropped,
                ..context()
            };
            assert_eq!(
                map_block(context, &json!({ "type": "text", "text": "hello" })),
                BlockRow::None,
                "{dropped} text is dropped by both readers already"
            );
        }
    }

    #[test]
    fn a_tool_use_starts_a_call_in_progress() {
        let activity = only(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": "Bash",
                "input": { "command": "ls -la" }
            }),
        ));
        assert_eq!(activity.activity_id, "msg-1:3");
        assert_eq!(activity.kind, ActivityKind::ToolCall);
        assert_eq!(activity.status, ActivityStatus::InProgress);
        assert_eq!(activity.tool_use_id.as_deref(), Some("tu-1"));
        assert_eq!(activity.summary, "Bash");
        assert!(!activity.is_error);
        assert_eq!(activity.payload["itemType"], "command_execution");
        assert_eq!(activity.payload["status"], "inProgress");
        assert_eq!(activity.payload["command"], "ls -la");
        assert_eq!(activity.payload["data"]["toolCallId"], "tu-1");
        assert_eq!(activity.payload["input"]["command"], "ls -la");
    }

    #[test]
    fn a_tool_use_with_no_id_is_dropped_whole() {
        assert_eq!(
            map_block(context(), &json!({ "type": "tool_use", "name": "Bash" })),
            BlockRow::None
        );
    }

    #[test]
    fn a_result_completes_the_call_it_names() {
        assert_eq!(
            map_block(
                context(),
                &json!({ "type": "tool_result", "tool_use_id": "tu-1", "content": "done" })
            ),
            BlockRow::ToolOutcome(ToolOutcome {
                task_id: "task-1".into(),
                tool_use_id: "tu-1".into(),
                status: ActivityStatus::Completed,
                result: json!("done"),
                // Not the block's to carry — the caller attaches it.
                typed_result: None,
                is_error: false,
            })
        );
    }

    #[test]
    fn an_errored_result_fails_the_call() {
        let BlockRow::ToolOutcome(outcome) = map_block(
            context(),
            &json!({
                "type": "tool_result",
                "tool_use_id": "tu-1",
                "content": "boom",
                "is_error": true
            }),
        ) else {
            panic!("a tool_result is an outcome");
        };
        assert_eq!(outcome.status, ActivityStatus::Failed);
        assert!(outcome.is_error);
    }

    #[test]
    fn a_result_naming_no_call_answers_nothing() {
        assert_eq!(
            map_block(context(), &json!({ "type": "tool_result", "content": "x" })),
            BlockRow::None
        );
    }

    /// The outcome carries the result whole; the cut copy is `detail`, beside
    /// it.
    #[test]
    fn a_long_result_is_kept_uncapped() {
        let long = "x".repeat(10_000);
        let BlockRow::ToolOutcome(outcome) = map_block(
            context(),
            &json!({ "type": "tool_result", "tool_use_id": "tu-1", "content": long }),
        ) else {
            panic!("a tool_result is an outcome");
        };
        assert_eq!(outcome.result.as_str().map(str::len), Some(10_000));
    }

    #[test]
    fn the_detail_beside_it_is_cut_and_marked() {
        let cut = detail(&json!("x".repeat(10_000))).expect("visible output");
        assert!(cut.starts_with(&"x".repeat(DETAIL_LIMIT)));
        assert!(cut.ends_with("… truncated"));
        assert!(cut.len() < DETAIL_LIMIT + 32);
    }

    #[test]
    fn a_short_result_is_its_own_detail() {
        assert_eq!(detail(&json!("done")).as_deref(), Some("done"));
    }

    /// A multi-byte character straddling the limit is not sliced through.
    #[test]
    fn the_cut_lands_on_a_character_boundary() {
        let cut = detail(&json!("é".repeat(DETAIL_LIMIT))).expect("visible output");
        assert!(cut.ends_with("… truncated"));
    }

    #[test]
    fn a_result_with_no_visible_output_previews_nothing() {
        assert_eq!(detail(&json!({ "structured": true })), None);
    }

    /// A tool result comes back as a bare string for most tools and as a block
    /// list for the ones that return structured output.
    #[test]
    fn a_structured_result_previews_its_text_block() {
        let structured = json!([
            { "type": "image", "source": {} },
            { "type": "text", "text": "3 files changed" }
        ]);
        assert_eq!(detail(&structured).as_deref(), Some("3 files changed"));
    }

    #[test]
    fn thinking_stores_its_label_and_the_thought() {
        for kind in ["thinking", "redacted_thinking"] {
            let activity = only(map_block(
                context(),
                &json!({ "type": kind, "thinking": "a private thought" }),
            ));
            assert_eq!(activity.kind, ActivityKind::Thinking);
            assert_eq!(activity.summary, THINKING_SUMMARY);
            assert_eq!(
                activity.payload,
                json!({ "summary": THINKING_SUMMARY, "text": "a private thought" })
            );
        }
    }

    #[test]
    fn a_long_thought_is_cut_on_a_character_boundary() {
        let long = "é".repeat(THINKING_TEXT_MAX_BYTES);
        let activity = only(map_block(
            context(),
            &json!({ "type": "thinking", "thinking": long }),
        ));
        let text = activity.payload["text"].as_str().unwrap();
        assert!(text.len() <= THINKING_TEXT_MAX_BYTES);
        assert_eq!(activity.payload["truncated"], json!(true));
    }

    /// The four `UNRENDERED_BLOCK_TYPES`, and one type nothing has ever seen.
    /// Two of the four are thinking, which has a row; the other two become
    /// nothing.
    #[test]
    fn a_block_this_conversation_does_not_store_is_silent() {
        for unstored in [
            "server_tool_use",
            "web_search_tool_result",
            "image",
            "a_new_sdk_block",
        ] {
            assert_eq!(
                map_block(context(), &json!({ "type": unstored })),
                BlockRow::None,
                "{unstored} is a gap in this projection, not a fault"
            );
        }
    }

    /// The identity a per-block event produces must equal the identity the same
    /// block produces inside a whole assembled array — that is what makes
    /// redelivery in either shape a no-op.
    #[test]
    fn a_block_has_the_same_identity_whichever_shape_it_arrived_in() {
        let whole = map_content(
            BlockContext {
                block_index: 0,
                source_block: 0,
                ..context()
            },
            &json!([
                { "type": "text", "text": "first" },
                { "type": "tool_use", "id": "tu-1", "name": "Bash" },
                { "type": "thinking" }
            ]),
        );
        let per_block = map_content(
            BlockContext {
                block_index: 2,
                source_block: 2,
                ..context()
            },
            &json!([{ "type": "thinking" }]),
        );

        assert_eq!(whole.text[0].block_index, 0);
        assert_eq!(whole.activities[0].activity_id, "msg-1:1");
        assert_eq!(whole.activities[1].activity_id, "msg-1:2");
        assert_eq!(per_block.activities[0].activity_id, "msg-1:2");
    }

    #[test]
    fn a_message_written_as_a_bare_string_is_still_text() {
        let write = map_content(context(), &json!("just text"));
        assert_eq!(write.text.len(), 1);
        assert_eq!(write.text[0].text, "just text");
    }

    #[test]
    fn a_subagents_activity_carries_the_call_that_spawned_it() {
        let context = BlockContext {
            parent_uuid: Some("agent-tool-1"),
            ..context()
        };
        let activity = only(map_block(
            context,
            &json!({ "type": "tool_use", "id": "tu-1", "name": "Bash" }),
        ));
        assert_eq!(activity.parent_uuid.as_deref(), Some("agent-tool-1"));
    }

    // ── joined rows ─────────────────────────────────────────────────

    /// The panel is derived from a row of its own; the call is still stored as
    /// a call, so every tool renderer still finds it where it always was.
    #[test]
    fn a_question_is_stored_as_a_call_and_as_the_question_it_asked() {
        let written = rows(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": QUESTION_TOOL,
                "input": { "questions": [{
                    "question": "Which store?",
                    "header": "Storage",
                    "multiSelect": true,
                    "options": [
                        { "label": "Postgres", "description": "The one already deployed." },
                        { "label": "Redis" },
                    ],
                }] }
            }),
        ));

        assert_eq!(written.len(), 2);
        assert_eq!(written[0].kind, ActivityKind::ToolCall);
        assert_eq!(written[0].activity_id, "msg-1:3");

        let asked = &written[1];
        assert_eq!(asked.kind, ActivityKind::UserInputRequested);
        assert_eq!(asked.activity_id, "msg-1:3:user-input");
        // The joined row happened at the block the call is, not after it.
        assert_eq!(asked.ordinal, written[0].ordinal);
        assert_eq!(asked.payload["requestId"], "tu-1");
        // Only the call names the call: `(task_id, tool_use_id)` is unique, so
        // a joined row that repeated it would reject the whole write.
        assert_eq!(written[0].tool_use_id.as_deref(), Some("tu-1"));
        assert!(asked.tool_use_id.is_none());
        let question = &asked.payload["questions"][0];
        // The id *is* the question text: that is the key the CLI looks an
        // answer up by, so a synthetic one answers nothing.
        assert_eq!(question["id"], "Which store?");
        assert_eq!(question["header"], "Storage");
        assert_eq!(question["multiSelect"], true);
        assert_eq!(
            question["options"][0]["description"],
            "The one already deployed."
        );
        // The client's parser drops an option with no description, and the API
        // shape leaves it optional, so the label stands in for one.
        assert_eq!(question["options"][1]["description"], "Redis");
    }

    /// A question with no header still has to parse, so one is filled in.
    #[test]
    fn a_question_with_no_header_is_numbered() {
        let written = rows(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": QUESTION_TOOL,
                "input": { "questions": [
                    { "question": "First?", "options": [{ "label": "Yes" }] },
                    { "question": "Second?", "options": [{ "label": "Yes" }] },
                ] }
            }),
        ));
        let questions = &written[1].payload["questions"];
        assert_eq!(questions[0]["header"], "Question 1");
        assert_eq!(questions[1]["header"], "Question 2");
    }

    /// Nothing to ask means no panel to derive. The call is still a call.
    #[test]
    fn a_question_that_asked_nothing_joins_no_row() {
        for input in [json!({}), json!({ "questions": [] })] {
            let written = rows(map_block(
                context(),
                &json!({
                    "type": "tool_use", "id": "tu-1", "name": QUESTION_TOOL, "input": input
                }),
            ));
            assert_eq!(written.len(), 1, "only the call itself");
        }
    }

    /// A proposed plan parks the run on a call only an answer releases, and the
    /// answer panel is the one surface that can send one — so the plan is
    /// asked, not merely shown.
    #[test]
    fn a_proposed_plan_asks_whether_to_approve_it() {
        let written = rows(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": PLAN_TOOL,
                "input": { "plan": "1. Do the thing" }
            }),
        ));
        assert_eq!(written.len(), 2);
        let asked = &written[1];
        assert_eq!(asked.kind, ActivityKind::UserInputRequested);
        assert_eq!(asked.payload["requestId"], "tu-1");
        let options = &asked.payload["questions"][0]["options"];
        assert_eq!(options[0]["label"], PLAN_APPROVE);
        assert_eq!(options[1]["label"], PLAN_REJECT);
    }

    /// Both plan trackers carry a three-state status per step, and *in
    /// progress* is the state a plan panel exists to show.
    #[test]
    fn a_plan_tracker_keeps_each_steps_own_state() {
        let claude = rows(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": TODO_TOOL,
                "input": { "todos": [
                    { "content": "first", "status": "completed" },
                    { "content": "second", "status": "in_progress" },
                    { "content": "third", "status": "pending" },
                ] }
            }),
        ));
        let codex = rows(map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": CODEX_TOOL_TODO_LIST,
                "input": { "items": [
                    { "text": "first", "completed": true, "status": "completed" },
                    { "text": "second", "completed": false, "status": "inProgress" },
                    { "text": "third", "completed": false, "status": "pending" },
                ] }
            }),
        ));

        let expected = json!([
            { "step": "first", "status": "completed" },
            { "step": "second", "status": "inProgress" },
            { "step": "third", "status": "pending" },
        ]);
        for written in [claude, codex] {
            assert_eq!(written.len(), 2);
            assert_eq!(written[1].kind, ActivityKind::PlanUpdated);
            assert_eq!(written[1].activity_id, "msg-1:3:plan");
            assert_eq!(written[1].payload["plan"], expected);
            assert!(written[1].tool_use_id.is_none());
        }
    }

    /// An entry that states no status says only done or not done, and that is
    /// all it is read as.
    #[test]
    fn a_step_with_no_status_falls_back_to_its_boolean() {
        assert_eq!(step_status(&json!({ "completed": true })), "completed");
        assert_eq!(step_status(&json!({ "completed": false })), "pending");
        assert_eq!(step_status(&json!({})), "pending");
    }

    /// An empty tracker is a panel with nothing in it.
    #[test]
    fn an_empty_plan_joins_no_row() {
        let written = rows(map_block(
            context(),
            &json!({
                "type": "tool_use", "id": "tu-1", "name": TODO_TOOL, "input": { "todos": [] }
            }),
        ));
        assert_eq!(written.len(), 1);
    }

    /// `(task_id, tool_use_id)` is unique, and one Message's rows write in one
    /// transaction — so naming a call twice would persist none of the Message,
    /// taking the prose and calls down with the panel.
    #[test]
    fn one_message_names_each_call_at_most_once() {
        let write = map_content(
            context(),
            &json!([
                { "type": "text", "text": "on it" },
                { "type": "tool_use", "id": "tu-1", "name": TODO_TOOL, "input": { "todos": [
                    { "content": "first", "status": "pending" },
                ] } },
                { "type": "tool_use", "id": "tu-2", "name": QUESTION_TOOL, "input": { "questions": [
                    { "question": "Which store?", "options": [{ "label": "Postgres" }] },
                ] } },
                { "type": "tool_use", "id": "tu-3", "name": PLAN_TOOL, "input": { "plan": "1. Go" } },
                { "type": "tool_use", "id": "tu-4", "name": "Bash" },
            ]),
        );

        let named: Vec<&str> = write
            .activities
            .iter()
            .filter_map(|activity| activity.tool_use_id.as_deref())
            .collect();
        assert_eq!(named, ["tu-1", "tu-2", "tu-3", "tu-4"]);
        // The panels are still derived, and still joined by `requestId`.
        assert_eq!(write.activities.len(), 7);
    }

    /// Every other call is one row, as it always was.
    #[test]
    fn an_ordinary_call_joins_nothing() {
        let written = rows(map_block(
            context(),
            &json!({ "type": "tool_use", "id": "tu-1", "name": "Bash" }),
        ));
        assert_eq!(written.len(), 1);
    }

    /// `requestId` joins both rows; the `tool_use_id` column stays empty, same
    /// as a joined row's, since the answer settles the call via the outcome
    /// beside it.
    #[test]
    fn an_answer_is_stored_against_the_call_it_answers() {
        let resolved = user_input_resolved(
            "task-1",
            "msg-9",
            "tu-1",
            &json!({ "Which store?": "Postgres" }),
        );
        assert_eq!(resolved.kind, ActivityKind::UserInputResolved);
        assert_eq!(resolved.activity_id, "msg-9:0:user-input-resolved");
        assert!(resolved.tool_use_id.is_none());
        assert_eq!(resolved.payload["requestId"], "tu-1");
        assert_eq!(resolved.payload["answers"]["Which store?"], "Postgres");
    }

    /// The meter shows a count with no percentage rather than a percentage of a
    /// guess, so a reading with no maximum carries none.
    #[test]
    fn a_context_window_reading_carries_a_maximum_only_when_one_was_reported() {
        let known = context_window_updated("evt-1", "task-1", 1_000, Some(200_000));
        assert_eq!(known.kind, ActivityKind::ContextWindowUpdated);
        assert_eq!(known.activity_id, "evt-1");
        assert_eq!(known.payload["usedTokens"], 1_000);
        assert_eq!(known.payload["maxTokens"], 200_000);
        assert!(known.message_uuid.is_none());

        let unknown = context_window_updated("evt-2", "task-1", 1_000, None);
        assert!(unknown.payload.get("maxTokens").is_none());
    }

    /// The set is closed in code: a kind that round-trips through the column is
    /// one this build knows, and one it does not is a compile error rather than
    /// a row nothing can read.
    #[test]
    fn every_kind_round_trips_through_the_column() {
        use std::str::FromStr;
        for kind in ActivityKind::ALL {
            assert_eq!(
                ActivityKind::from_str(kind.as_str()),
                Ok(*kind),
                "{} round-trips",
                kind.as_str()
            );
        }
    }

    fn sandbox_payload(lifecycle_event: Option<&str>) -> SandboxEventPayload {
        SandboxEventPayload {
            lifecycle_event: lifecycle_event.map(str::to_string),
            ..SandboxEventPayload::default()
        }
    }

    #[test]
    fn a_sandbox_status_is_identified_by_its_event() {
        let activity = sandbox_activity(
            "evt-1",
            "task-1",
            "ready",
            false,
            &sandbox_payload(Some("pod_ready")),
        );
        assert_eq!(activity.activity_id, "evt-1");
        assert_eq!(activity.kind, ActivityKind::SandboxStatus);
        assert_eq!(activity.summary, "Ready");
        assert!(activity.message_uuid.is_none());
        assert!(!activity.is_error);
    }

    /// A stall the run recovers from is the third severity: an amber warning,
    /// not a status and not a fatal error.
    #[test]
    fn a_sandbox_stall_is_a_warning() {
        let activity = sandbox_activity(
            "evt-slow",
            "task-1",
            "initializing",
            false,
            &sandbox_payload(Some("startup_slow")),
        );
        assert_eq!(activity.kind, ActivityKind::SandboxWarning);
        assert_eq!(activity.summary, "Slow startup");
        assert_eq!(activity.status, ActivityStatus::Completed);
        assert!(!activity.is_error, "a warning is not an error on the row");
    }

    #[test]
    fn a_sandbox_error_is_labelled_and_flagged_from_its_lifecycle_event() {
        let mut payload = sandbox_payload(Some("container_oom_killed"));
        payload.message = "Out of memory (OOMKilled)".to_string();
        payload.exit_code = Some(137);
        payload.terminated_reason = Some("OOMKilled".to_string());
        payload.node_name = Some("ip-10-0-0-1".to_string());
        let activity = sandbox_activity("evt-2", "task-1", "error", true, &payload);
        assert_eq!(activity.kind, ActivityKind::SandboxError);
        assert_eq!(activity.summary, "Out of memory");
        assert_eq!(activity.status, ActivityStatus::Failed);
        assert!(activity.is_error);
        // Message first, then one line per present field in `buildChips` order.
        assert_eq!(
            activity.payload["detail"],
            "Out of memory (OOMKilled)\nnode ip-10-0-0-1\nexit 137\nreason OOMKilled"
        );
    }

    /// A `pod_scheduled` reading composes its node and its pending duration
    /// into the detail, the compact duration format and all.
    #[test]
    fn a_scheduled_reading_composes_node_and_pending_into_its_detail() {
        let mut payload = sandbox_payload(Some("pod_scheduled"));
        payload.message = "Scheduled onto a node".to_string();
        payload.node_name = Some("i-0040f860ad3bd3487".to_string());
        payload.pending_seconds = Some(37);
        let activity = sandbox_activity("evt-s", "task-1", "initializing", false, &payload);
        assert_eq!(activity.summary, "Scheduled");
        assert_eq!(activity.kind, ActivityKind::SandboxStatus);
        assert_eq!(
            activity.payload["detail"],
            "Scheduled onto a node\nnode i-0040f860ad3bd3487\npending 37s"
        );
    }

    /// `lifecycle_descriptor` names one event of each severity; the coarse
    /// fallback and the unknown-slug fallback both take the event's error-ness.
    #[test]
    fn the_lifecycle_vocabulary_and_its_fallbacks() {
        assert_eq!(
            lifecycle_descriptor("pod_ready"),
            Some(("Ready", Severity::Status))
        );
        assert_eq!(
            lifecycle_descriptor("startup_slow"),
            Some(("Slow startup", Severity::Warning))
        );
        assert_eq!(
            lifecycle_descriptor("container_oom_killed"),
            Some(("Out of memory", Severity::Error))
        );
        assert_eq!(lifecycle_descriptor("brand_new_slug"), None);

        // No `lifecycleEvent` and no message: labelled from the coarse status,
        // severity from the sub-type's error-ness.
        let coarse = sandbox_activity("e", "task-1", "initializing", false, &sandbox_payload(None));
        assert_eq!(coarse.summary, "Initializing");
        assert_eq!(coarse.kind, ActivityKind::SandboxStatus);
        let coarse_error = sandbox_activity("e", "task-1", "error", true, &sandbox_payload(None));
        assert_eq!(coarse_error.summary, "Error");
        assert_eq!(coarse_error.kind, ActivityKind::SandboxError);

        // An unknown slug renders as itself, spaced, at the sub-type's severity.
        let unknown = sandbox_activity(
            "e",
            "task-1",
            "initializing",
            false,
            &sandbox_payload(Some("brand_new_slug")),
        );
        assert_eq!(unknown.summary, "brand new slug");
        assert_eq!(unknown.kind, ActivityKind::SandboxStatus);
    }

    /// The Sandbox's init publishes its steps under one sub-type, told apart
    /// only by message. Labelling by sub-type collapsed them into one
    /// `Initializing` after another, dropping the rest.
    #[test]
    fn each_init_step_is_its_own_row_labelled_by_what_it_says() {
        let step = |message: &str| {
            let mut payload = sandbox_payload(None);
            payload.message = message.to_string();
            sandbox_activity("evt", "task-1", "initializing", false, &payload)
        };

        let cloned = step("Cloned repository moatless (main)");
        assert_eq!(cloned.summary, "Cloned repository moatless (main)");
        assert_eq!(cloned.kind, ActivityKind::SandboxStatus);
        // The label is the message, so the detail does not say it twice.
        assert!(cloned.payload.get("detail").is_none());

        let steps = [
            "Fetched credentials from backend",
            "Configured git authentication",
            "Cloned repository moatless (main)",
            "Workspace setup complete",
        ];
        let summaries: Vec<String> = steps.iter().map(|m| step(m).summary).collect();
        assert_eq!(
            summaries, steps,
            "no two steps share a label to collapse on"
        );
    }

    /// A message that only repeats what the vocabulary already named is still a
    /// detail line — the two differ, and the fields ride under it as before.
    #[test]
    fn a_named_event_keeps_its_message_as_detail() {
        let mut payload = sandbox_payload(Some("pod_scheduled"));
        payload.message = "Scheduled onto a node".to_string();
        let activity = sandbox_activity("evt", "task-1", "initializing", false, &payload);
        assert_eq!(activity.summary, "Scheduled");
        assert_eq!(activity.payload["detail"], "Scheduled onto a node");
    }

    #[test]
    fn compact_durations_read_the_way_the_frontend_writes_them() {
        assert_eq!(format_duration_seconds(37), "37s");
        assert_eq!(format_duration_seconds(364), "6m 4s");
        assert_eq!(format_duration_seconds(120), "2m");
        assert_eq!(format_duration_seconds(4_800), "1h 20m");
        assert_eq!(format_duration_seconds(3_600), "1h");
        assert_eq!(format_duration_seconds(-5), "0s");
    }

    #[test]
    fn item_type_claims_only_the_two_shapes_with_a_renderer() {
        assert_eq!(item_type("Bash"), "command_execution");
        assert_eq!(item_type("BashOutput"), "command_execution");
        assert_eq!(item_type("Edit"), "file_change");
        assert_eq!(item_type("Write"), "file_change");
        assert_eq!(item_type("NotebookEdit"), "file_change");
        assert_eq!(item_type("Grep"), "dynamic_tool_call");
        assert_eq!(item_type("mcp__linear__search"), "dynamic_tool_call");
    }

    #[test]
    fn result_text_reads_a_string_or_the_first_text_part() {
        assert_eq!(result_text(&json!("plain")), Some("plain"));
        assert_eq!(
            result_text(&json!([
                { "type": "image" },
                { "type": "text", "text": "structured" }
            ])),
            Some("structured")
        );
        assert_eq!(result_text(&json!({ "shape": "unknown" })), None);
    }

    #[test]
    fn kind_and_status_round_trip_through_their_columns() {
        for kind in [
            ActivityKind::ToolCall,
            ActivityKind::Thinking,
            ActivityKind::SandboxStatus,
            ActivityKind::SandboxError,
        ] {
            assert_eq!(kind.as_str().parse::<ActivityKind>(), Ok(kind));
        }
        assert!("run_outcome".parse::<ActivityKind>().is_err());
    }

    #[test]
    fn an_empty_write_is_one_nothing_needs_a_transaction_for() {
        assert!(ConversationWrite::default().is_empty());
        let write = map_content(context(), &json!([{ "type": "image" }]));
        assert!(write.is_empty());
    }
}

//! What one arriving content block becomes.
//!
//! A pure function from a block plus the context it arrived in to the row it
//! changes. It is the seam this whole change is tested at: the live writer and
//! the migration both go through it, so the two cannot drift into two different
//! answers for the same block.

use serde_json::{Map, Value, json};

use crate::message::models::MessageType;

/// What the writer already knows about a block before it decides anything.
#[derive(Debug, Clone, Copy)]
pub struct BlockContext<'a> {
    pub task_id: &'a str,
    pub message_uuid: &'a str,
    /// The `Agent` tool-use id this Message's subagent transcript hangs under,
    /// or `None` for the Task's own conversation.
    pub parent_uuid: Option<&'a str>,
    /// The Message's type, which is what decides whether prose is a Message at
    /// all. Both readers already drop `system`, `result` and `error` prose; this
    /// is where storage stops writing what they throw away.
    pub message_type: MessageType,
    /// The block's native index within its Message, from ADR-0002's
    /// `blockIndex`.
    pub block_index: usize,
}

/// The row-level change one block represents.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockRow {
    /// Prose, spliced into the Message's text at `block_index`.
    Text(TextPart),
    /// A tool call or a stretch of thinking, written as its own row.
    Activity(Box<ActivityRow>),
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
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    pub is_error: bool,
}

/// The update a `tool_result` block makes to the call it answers.
///
/// It carries no `message_uuid` on purpose: a result arrives on a *later*
/// Message than its call, so it cannot derive the call's
/// `{message_uuid}:{block_index}` key and finds the row by
/// `(task_id, tool_use_id)` instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub task_id: String,
    pub tool_use_id: String,
    pub status: ActivityStatus,
    /// The result whole. What the Moatless frontend renders, and what
    /// [`detail`] is cut from.
    pub result: Value,
    /// The transport's own structured view of the same outcome — the SDK's
    /// `toolUseResult`, which is what the codex typed schemas validate against.
    /// It is not the block's to carry: it is a field of the Message the block
    /// arrived on, so the caller attaches it after mapping. `None` where the
    /// transport reported none.
    pub typed_result: Option<Value>,
    pub is_error: bool,
}

/// The closed set. A value outside it is a bug, not an extension point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    ToolCall,
    Thinking,
    SandboxStatus,
    SandboxError,
}

impl ActivityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::Thinking => "thinking",
            Self::SandboxStatus => "sandbox_status",
            Self::SandboxError => "sandbox_error",
        }
    }
}

impl std::str::FromStr for ActivityKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "tool_call" => Ok(Self::ToolCall),
            "thinking" => Ok(Self::Thinking),
            "sandbox_status" => Ok(Self::SandboxStatus),
            "sandbox_error" => Ok(Self::SandboxError),
            other => Err(format!("unknown activity kind: {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityStatus {
    InProgress,
    Completed,
    Failed,
}

impl ActivityStatus {
    /// How the column stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    /// How the T3 contract spells it. Two spellings because the column is this
    /// codebase's vocabulary and the payload is the client's — `projection.rs`
    /// has always written `inProgress` into the payload it sends.
    pub fn wire(self) -> &'static str {
        match self {
            Self::InProgress => "inProgress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Everything one Message's content array changes, in one value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationWrite {
    pub text: Vec<TextPart>,
    pub activities: Vec<ActivityRow>,
    pub outcomes: Vec<ToolOutcome>,
    /// The Message's reported token usage, admitted once per Message.
    pub usage: Option<(i32, i32)>,
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
/// `context.block_index` is the index of the array's *first* block: 0 for a
/// whole assembled array, and the event's own `blockIndex` for the one-element
/// array a per-block source event carries (ADR-0002). That is what keeps a
/// block's identity — and therefore its activity's id — the same whichever
/// shape it arrived in.
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
            ..context
        };
        match map_block(context, block) {
            BlockRow::Text(part) => write.text.push(part),
            BlockRow::Activity(activity) => write.activities.push(*activity),
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
        Some("thinking" | "redacted_thinking") => thinking(context),
        // Silently, as `projection.rs` already does: an unrecognised block is an
        // SDK feature this conversation does not render, not a fault.
        _ => BlockRow::None,
    }
}

/// Prose, but only where prose is a Message.
///
/// The narrowing to `user` and `assistant` is not a new rule. It is the rule
/// both readers already apply on every read: a `system` row is dropped outright,
/// and `result` and `error` become the run's own closing line rather than
/// something somebody said.
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
    matches!(
        message_type,
        MessageType::User | MessageType::Assistant
    )
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

    BlockRow::Activity(Box::new(ActivityRow {
        activity_id: activity_id(context),
        task_id: context.task_id.to_string(),
        message_uuid: Some(context.message_uuid.to_string()),
        parent_uuid: context.parent_uuid.map(str::to_string),
        kind: ActivityKind::ToolCall,
        status,
        tool_use_id: Some(tool_use_id.to_string()),
        summary: name.to_string(),
        payload: Value::Object(payload),
        is_error: false,
    }))
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

/// Thinking keeps only its label.
///
/// Nothing renders the thought: the Moatless frontend has no thinking branch,
/// and the T3 surface deliberately carries only the word. Storing the label is
/// what keeps `task.progress` reaching that client; storing the text would be
/// 301 MB nothing reads.
fn thinking(context: BlockContext<'_>) -> BlockRow {
    BlockRow::Activity(Box::new(ActivityRow {
        activity_id: activity_id(context),
        task_id: context.task_id.to_string(),
        message_uuid: Some(context.message_uuid.to_string()),
        parent_uuid: context.parent_uuid.map(str::to_string),
        kind: ActivityKind::Thinking,
        status: ActivityStatus::Completed,
        tool_use_id: None,
        summary: THINKING_SUMMARY.to_string(),
        payload: json!({ "summary": THINKING_SUMMARY }),
        is_error: false,
    }))
}

/// A Sandbox lifecycle change, which happened on no Message.
///
/// Its identity is the event's own id rather than a block position, so the same
/// event redelivered writes nothing.
pub fn sandbox_activity(
    event_id: &str,
    task_id: &str,
    status: &str,
    detail: Option<&str>,
    is_error: bool,
) -> ActivityRow {
    let mut payload = Map::new();
    payload.insert("summary".into(), Value::String(status.to_string()));
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        payload.insert("detail".into(), Value::String(detail.to_string()));
    }
    ActivityRow {
        activity_id: event_id.to_string(),
        task_id: task_id.to_string(),
        message_uuid: None,
        parent_uuid: None,
        kind: if is_error {
            ActivityKind::SandboxError
        } else {
            ActivityKind::SandboxStatus
        },
        status: if is_error {
            ActivityStatus::Failed
        } else {
            ActivityStatus::Completed
        },
        tool_use_id: None,
        summary: status.to_string(),
        payload: Value::Object(payload),
        is_error,
    }
}

/// `{message_uuid}:{block_index}` — the id `projection.rs` already emits, which
/// is why the T3 client sees no change.
fn activity_id(context: BlockContext<'_>) -> String {
    format!("{}:{}", context.message_uuid, context.block_index)
}

/// Which of the contract's closed set of tool shapes a Moatless tool call is.
///
/// The set names how the client *renders* a call, so only the two shapes with a
/// dedicated renderer are worth claiming. Guessing from a free-text tool name is
/// how a search tool ends up rendered as a file diff.
pub fn item_type(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "BashOutput" => "command_execution",
        "Edit" | "Write" | "NotebookEdit" => "file_change",
        _ => "dynamic_tool_call",
    }
}

/// A tool result's visible output.
///
/// The block's `content` is a string for most tools and a list of content blocks
/// for the ones that return structured output; only the first text block of the
/// latter is shown, which is what the work log has room for.
pub fn result_text(result: &Value) -> Option<&str> {
    if let Some(text) = result.as_str() {
        return Some(text);
    }
    result
        .as_array()?
        .iter()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|part| part.get("text").and_then(Value::as_str))
}

/// Tool output long enough to be a file rather than a preview.
///
/// The work log shows a few lines and expands to the rest, but on the T3 surface
/// the rest still crosses the socket, and that surface re-sends its whole log
/// every time the conversation moves. A single `Read` of a large file would put
/// the file in every one of those sends.
pub const DETAIL_LIMIT: usize = 2_000;

/// What a settled call shows before it is expanded, cut to [`DETAIL_LIMIT`].
///
/// `None` where the result has no visible output to preview. The cut is marked
/// rather than silent, and lands on a character boundary — the text is a `str`,
/// and slicing mid-codepoint panics.
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
        }
    }

    #[test]
    fn text_is_prose_on_the_message_it_arrived_on() {
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

    /// Decision 13: only `user` and `assistant` prose survives, because that is
    /// the exclusion both readers already apply on every read.
    #[test]
    fn prose_on_a_row_nobody_said_is_not_stored() {
        for dropped in [MessageType::System, MessageType::Result, MessageType::Error] {
            let context = BlockContext {
                message_type: dropped,
                ..context()
            };
            assert_eq!(
                map_block(context, &json!({ "type": "text", "text": "hello" })),
                BlockRow::None,
                "{dropped} prose is dropped by both readers already"
            );
        }
    }

    #[test]
    fn a_tool_use_starts_a_call_in_progress() {
        let BlockRow::Activity(activity) = map_block(
            context(),
            &json!({
                "type": "tool_use",
                "id": "tu-1",
                "name": "Bash",
                "input": { "command": "ls -la" }
            }),
        ) else {
            panic!("a tool_use is an activity");
        };
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

    /// The outcome carries the result whole; the cut copy is `detail`, beside it.
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
    fn thinking_keeps_only_its_label() {
        for kind in ["thinking", "redacted_thinking"] {
            let BlockRow::Activity(activity) = map_block(
                context(),
                &json!({ "type": kind, "thinking": "a long private thought" }),
            ) else {
                panic!("{kind} is an activity");
            };
            assert_eq!(activity.kind, ActivityKind::Thinking);
            assert_eq!(activity.summary, THINKING_SUMMARY);
            assert_eq!(activity.payload, json!({ "summary": THINKING_SUMMARY }));
            assert!(
                !activity.payload.to_string().contains("private thought"),
                "the thought itself is not stored"
            );
        }
    }

    /// The four `UNRENDERED_BLOCK_TYPES` the old block writer skipped, and one
    /// type nothing has ever seen. Two of the four are thinking, which now has a
    /// row; the other two still become nothing.
    #[test]
    fn a_block_this_conversation_does_not_store_is_silent() {
        for unstored in ["server_tool_use", "web_search_tool_result", "image", "a_new_sdk_block"] {
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
    fn a_message_written_as_a_bare_string_is_still_prose() {
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
        let BlockRow::Activity(activity) =
            map_block(context, &json!({ "type": "tool_use", "id": "tu-1", "name": "Bash" }))
        else {
            panic!("a tool_use is an activity");
        };
        assert_eq!(activity.parent_uuid.as_deref(), Some("agent-tool-1"));
    }

    #[test]
    fn a_sandbox_status_is_identified_by_its_event() {
        let activity = sandbox_activity("evt-1", "task-1", "Ready", None, false);
        assert_eq!(activity.activity_id, "evt-1");
        assert_eq!(activity.kind, ActivityKind::SandboxStatus);
        assert_eq!(activity.summary, "Ready");
        assert!(activity.message_uuid.is_none());
        assert!(!activity.is_error);
    }

    #[test]
    fn a_sandbox_error_keeps_the_detail_the_pod_watcher_published() {
        let detail = "Sandbox terminated: exit 137, OOMKilled, node ip-10-0-0-1";
        let activity = sandbox_activity("evt-2", "task-1", "Error", Some(detail), true);
        assert_eq!(activity.kind, ActivityKind::SandboxError);
        assert_eq!(activity.status, ActivityStatus::Failed);
        assert!(activity.is_error);
        assert_eq!(activity.payload["detail"], detail);
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

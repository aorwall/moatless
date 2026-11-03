//! Typed wire messages for `codex app-server --stdio`, and nothing else.
//!
//! Every struct is `#[serde(rename_all = "camelCase")]`, unlike the
//! `snake_case` `codex exec --json` stream this replaced.
//!
//! Unknown `ThreadItem` families and unknown `codexErrorInfo` values fall
//! through to a raw-JSON variant rather than failing the parse, so a newer
//! Codex release cannot break a running Turn.

use std::collections::HashMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

// =============================================================================
// Client → server requests
// =============================================================================

/// `initialize` — the first request on every connection. Its response must be
/// followed by the `initialized` notification before any thread work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParams {
    pub(crate) client_info: ClientInfo,
    /// Omitting this still permits `turn/steer` on codex-cli 0.145.0 (`codex
    /// features list` reports `steer removed true`); the harness sends `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) capabilities: Option<InitializeCapabilities>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientInfo {
    pub(crate) name: String,
    pub(crate) version: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeCapabilities {
    pub(crate) experimental_api: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResponse {
    pub(crate) user_agent: String,
    pub(crate) codex_home: String,
}

/// `thread/start` — opens a new Session. Every field is optional on the wire;
/// the harness supplies the `codex exec` spawn parity set (see the spec's
/// parity table).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

/// `thread/resume` — reopens a Session by id. `path` is accepted by the server
/// as an alternative locator; the harness always has the id.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

/// `config/read` resolves the effective provider for a working directory.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigReadParams {
    pub(crate) cwd: Option<String>,
    pub(crate) include_layers: bool,
}

/// The effective configuration returned by `config/read`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ConfigReadResponse {
    pub(crate) config: EffectiveConfig,
}

/// The provider selected by Codex's merged configuration layers.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EffectiveConfig {
    pub(crate) model_provider: Option<String>,
}

/// `thread/fork` — branches a new Session off an existing one at a cut.
///
/// `exclude_turns` excludes the cut Turn's own boundary so the fork's first
/// Turn is the resumed prompt, not a replay; omitting `last_turn_id` forks at
/// the source's head.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadForkParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    pub(crate) exclude_turns: bool,
}

/// The `thread` envelope shared by the `thread/start` and `thread/resume`
/// responses and by the `thread/started` notification. Only the Session id is
/// read; the rest of `Thread` is left on the wire.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadEnvelope {
    pub(crate) thread: ThreadRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadRef {
    pub(crate) id: String,
}

/// `turn/start` — runs one Turn on an open Session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartParams {
    pub(crate) thread_id: String,
    pub(crate) input: Vec<UserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartResponse {
    pub(crate) turn: Turn,
}

/// `turn/steer` — appends input to the Turn already running.
///
/// `expected_turn_id` is a precondition: empty, stale, or sent before
/// `turn/started` is rejected with `-32600` (see [`super`]).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnSteerParams {
    pub(crate) thread_id: String,
    pub(crate) expected_turn_id: String,
    pub(crate) input: Vec<UserInput>,
}

/// The Turn the steered input joined, which is the one it was sent for.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnSteerResponse {
    pub(crate) turn_id: String,
}

/// `turn/interrupt` — ends the running Turn. The Session survives; the Turn
/// terminates with [`TurnStatus::Interrupted`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnInterruptParams {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

/// One element of a Turn's `input` array. The harness only ever sends
/// [`UserInput::Text`]; the other variants exist because the same type is read
/// back off the `userMessage` item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum UserInput {
    Text {
        text: String,
    },
    Image {
        url: String,
    },
    LocalImage {
        path: String,
    },
    /// Any input kind this build does not model (`audio`, `skill`, …). Kept as
    /// a unit variant so reading back a `userMessage` never fails.
    #[serde(other)]
    Unsupported,
}

impl UserInput {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// Codex's own sandboxing mode. The task container is the sandbox boundary, so
/// the harness always asks for `danger-full-access` — the app-server equivalent
/// of `codex exec --dangerously-bypass-approvals-and-sandbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// When Codex asks the client to approve an action. The harness runs unattended
/// and always asks for `never`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AskForApproval {
    Untrusted,
    OnRequest,
    Never,
}

// =============================================================================
// Server → client notifications
// =============================================================================

// Every notification but `thread/started` carries the `threadId` it belongs to,
// since one connection can carry several (a sub-agent runs on its own thread).
// Each is `Option<String>`, read via [`ServerEvent::thread_id`] and matched by
// `CodexProcess::absorb`; a build that stops sending the field must fail open.
//
// `turnId` is not modelled: matching the thread already picks a single Turn.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartedNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) turn: Turn,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnCompletedNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) turn: Turn,
}

/// An `item/started` or `item/completed` payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) item: ThreadItem,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTokenUsageUpdatedNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) token_usage: ThreadTokenUsage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTokenUsage {
    /// Usage for the most recent model *request*, not the whole Turn.
    ///
    /// A Turn spanning three requests reports three of these; what it spent is
    /// `total`'s growth, and this is what currently occupies the context
    /// window.
    pub(crate) last: TokenUsageBreakdown,
    /// Usage accumulated over the whole Session, carried across
    /// `thread/resume`.
    pub(crate) total: TokenUsageBreakdown,
    /// The window the Session's model runs in. Absent where the app-server did
    /// not state one, which is a reading with no denominator rather than a
    /// zero.
    #[serde(default)]
    pub(crate) model_context_window: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "the field names are the wire's; dropping the postfix would need a rename on every one"
)]
pub(crate) struct TokenUsageBreakdown {
    pub(crate) input_tokens: i64,
    pub(crate) cached_input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) reasoning_output_tokens: i64,
}

/// The `error` notification — the same [`TurnError`] `turn/completed` will
/// carry, arriving first, plus whether Codex is retrying internally.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) error: TurnError,
    pub(crate) will_retry: bool,
}

/// `turn/plan/updated` — the running plan tracker, not the `plan` thread item;
/// this is what `codex exec --json` reported as `todo_list`.
///
/// The wire's `explanation` has no slot in `codex.todo_list` and stays there.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnPlanUpdatedNotification {
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    pub(crate) plan: Vec<TurnPlanStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnPlanStep {
    pub(crate) step: String,
    pub(crate) status: TurnPlanStepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TurnPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Unknown,
}

// =============================================================================
// Turn
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Turn {
    pub(crate) id: String,
    pub(crate) status: TurnStatus,
    /// Only populated when `status` is [`TurnStatus::Failed`].
    #[serde(default)]
    pub(crate) error: Option<TurnError>,
    /// Always empty with `items_view: NotLoaded` on `turn/completed`; content
    /// comes from streaming `item/completed` notifications, not this field.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the terminal replays nothing; the fixture test pins that and the harness relies on it"
        )
    )]
    #[serde(default)]
    pub(crate) items: Vec<ThreadItem>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "see `items` — the same measurement")
    )]
    #[serde(default = "TurnItemsView::full")]
    pub(crate) items_view: TurnItemsView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TurnItemsView {
    NotLoaded,
    Summary,
    Full,
    #[serde(other)]
    Unknown,
}

impl TurnItemsView {
    fn full() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnError {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) codex_error_info: Option<CodexErrorInfo>,
    #[serde(default)]
    pub(crate) additional_details: Option<String>,
}

/// The named failure cause on a failed Turn, read as a value rather than
/// matching substrings against free text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CodexErrorInfo {
    ContextWindowExceeded,
    SessionBudgetExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    CyberPolicy,
    InternalServerError,
    Unauthorized,
    BadRequest,
    ThreadRollbackFailed,
    SandboxError,
    Other,
    HttpConnectionFailed {
        http_status_code: Option<u16>,
    },
    ResponseStreamConnectionFailed {
        http_status_code: Option<u16>,
    },
    ResponseStreamDisconnected {
        http_status_code: Option<u16>,
    },
    ResponseTooManyFailedAttempts {
        http_status_code: Option<u16>,
    },
    ActiveTurnNotSteerable {
        turn_kind: NonSteerableTurnKind,
    },
    /// Unknown values land here with the raw JSON, so a new upstream code
    /// stays visible instead of collapsing into the default class.
    Unrecognized(serde_json::Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NonSteerableTurnKind {
    Review,
    Compact,
    #[serde(other)]
    Unknown,
}

impl<'de> Deserialize<'de> for CodexErrorInfo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;

        if let Some(name) = value.as_str() {
            return Ok(match name {
                "contextWindowExceeded" => Self::ContextWindowExceeded,
                "sessionBudgetExceeded" => Self::SessionBudgetExceeded,
                "usageLimitExceeded" => Self::UsageLimitExceeded,
                "serverOverloaded" => Self::ServerOverloaded,
                "cyberPolicy" => Self::CyberPolicy,
                "internalServerError" => Self::InternalServerError,
                "unauthorized" => Self::Unauthorized,
                "badRequest" => Self::BadRequest,
                "threadRollbackFailed" => Self::ThreadRollbackFailed,
                "sandboxError" => Self::SandboxError,
                "other" => Self::Other,
                _ => Self::Unrecognized(value),
            });
        }

        // The object variants are externally tagged: `{"<name>": {…}}`.
        let Some((name, payload)) = value
            .as_object()
            .filter(|map| map.len() == 1)
            .and_then(|map| map.iter().next())
        else {
            return Ok(Self::Unrecognized(value));
        };
        let status = || {
            payload
                .get("httpStatusCode")
                .and_then(serde_json::Value::as_u64)
                .and_then(|code| u16::try_from(code).ok())
        };
        Ok(match name.as_str() {
            "httpConnectionFailed" => Self::HttpConnectionFailed {
                http_status_code: status(),
            },
            "responseStreamConnectionFailed" => Self::ResponseStreamConnectionFailed {
                http_status_code: status(),
            },
            "responseStreamDisconnected" => Self::ResponseStreamDisconnected {
                http_status_code: status(),
            },
            "responseTooManyFailedAttempts" => Self::ResponseTooManyFailedAttempts {
                http_status_code: status(),
            },
            "activeTurnNotSteerable" => match payload.get("turnKind").cloned() {
                Some(kind) => Self::ActiveTurnNotSteerable {
                    turn_kind: serde_json::from_value(kind).map_err(D::Error::custom)?,
                },
                None => Self::Unrecognized(value.clone()),
            },
            _ => Self::Unrecognized(value.clone()),
        })
    }
}

// =============================================================================
// Thread items — the eighteen families
// =============================================================================

/// One item in a Turn, as delivered by `item/started` and `item/completed`.
///
/// Deserialization is hand-written, not `#[serde(tag = "type")]`, so an
/// unrecognised family keeps its name and payload in [`ThreadItem::Other`]
/// instead of failing the whole notification.
#[derive(Debug, Clone)]
pub(crate) enum ThreadItem {
    UserMessage(UserMessageItem),
    HookPrompt(HookPromptItem),
    AgentMessage(AgentMessageItem),
    Plan(PlanItem),
    Reasoning(ReasoningItem),
    CommandExecution(CommandExecutionItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    DynamicToolCall(DynamicToolCallItem),
    CollabAgentToolCall(CollabAgentToolCallItem),
    SubAgentActivity(SubAgentActivityItem),
    WebSearch(WebSearchItem),
    ImageView(ImageViewItem),
    Sleep(SleepItem),
    ImageGeneration(ImageGenerationItem),
    EnteredReviewMode(ReviewModeItem),
    ExitedReviewMode(ReviewModeItem),
    ContextCompaction(ContextCompactionItem),
    Other {
        type_name: String,
        id: String,
        raw: serde_json::Value,
    },
}

impl ThreadItem {
    /// The item id, which is also the tool-use id every canonical block pair is
    /// keyed on.
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::UserMessage(i) => &i.id,
            Self::HookPrompt(i) => &i.id,
            Self::AgentMessage(i) => &i.id,
            Self::Plan(i) => &i.id,
            Self::Reasoning(i) => &i.id,
            Self::CommandExecution(i) => &i.id,
            Self::FileChange(i) => &i.id,
            Self::McpToolCall(i) => &i.id,
            Self::DynamicToolCall(i) => &i.id,
            Self::CollabAgentToolCall(i) => &i.id,
            Self::SubAgentActivity(i) => &i.id,
            Self::WebSearch(i) => &i.id,
            Self::ImageView(i) => &i.id,
            Self::Sleep(i) => &i.id,
            Self::ImageGeneration(i) => &i.id,
            Self::EnteredReviewMode(i) | Self::ExitedReviewMode(i) => &i.id,
            Self::ContextCompaction(i) => &i.id,
            Self::Other { id, .. } => id,
        }
    }

    /// The wire `type` string, for logging an unhandled family without matching
    /// on every variant at the call site.
    pub(crate) fn type_name(&self) -> &str {
        match self {
            Self::UserMessage(_) => "userMessage",
            Self::HookPrompt(_) => "hookPrompt",
            Self::AgentMessage(_) => "agentMessage",
            Self::Plan(_) => "plan",
            Self::Reasoning(_) => "reasoning",
            Self::CommandExecution(_) => "commandExecution",
            Self::FileChange(_) => "fileChange",
            Self::McpToolCall(_) => "mcpToolCall",
            Self::DynamicToolCall(_) => "dynamicToolCall",
            Self::CollabAgentToolCall(_) => "collabAgentToolCall",
            Self::SubAgentActivity(_) => "subAgentActivity",
            Self::WebSearch(_) => "webSearch",
            Self::ImageView(_) => "imageView",
            Self::Sleep(_) => "sleep",
            Self::ImageGeneration(_) => "imageGeneration",
            Self::EnteredReviewMode(_) => "enteredReviewMode",
            Self::ExitedReviewMode(_) => "exitedReviewMode",
            Self::ContextCompaction(_) => "contextCompaction",
            Self::Other { type_name, .. } => type_name,
        }
    }
}

impl<'de> Deserialize<'de> for ThreadItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let type_name = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| D::Error::missing_field("type"))?
            .to_string();

        // Each arm deserializes from the full object; the item structs ignore
        // the `type` field they do not declare.
        macro_rules! typed {
            ($variant:ident) => {
                serde_json::from_value(value)
                    .map(Self::$variant)
                    .map_err(D::Error::custom)
            };
        }
        match type_name.as_str() {
            "userMessage" => typed!(UserMessage),
            "hookPrompt" => typed!(HookPrompt),
            "agentMessage" => typed!(AgentMessage),
            "plan" => typed!(Plan),
            "reasoning" => typed!(Reasoning),
            "commandExecution" => typed!(CommandExecution),
            "fileChange" => typed!(FileChange),
            "mcpToolCall" => typed!(McpToolCall),
            "dynamicToolCall" => typed!(DynamicToolCall),
            "collabAgentToolCall" => typed!(CollabAgentToolCall),
            "subAgentActivity" => typed!(SubAgentActivity),
            "webSearch" => typed!(WebSearch),
            "imageView" => typed!(ImageView),
            "sleep" => typed!(Sleep),
            "imageGeneration" => typed!(ImageGeneration),
            "enteredReviewMode" => typed!(EnteredReviewMode),
            "exitedReviewMode" => typed!(ExitedReviewMode),
            "contextCompaction" => typed!(ContextCompaction),
            _ => {
                let id = value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Self::Other {
                    type_name,
                    id,
                    raw: value,
                })
            }
        }
    }
}

/// What a steer produces, and what the Turn's own input is echoed back as.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserMessageItem {
    pub(crate) id: String,
    /// The echoed input. The harness renders nothing for this family — the text
    /// is already in the conversation as the Message that sent it (see the
    /// spec's *Data model*).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the echoed prompt is deliberately not rendered; the fixture test reads it"
        )
    )]
    pub(crate) content: Vec<UserInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HookPromptItem {
    pub(crate) id: String,
}

/// Assistant text. The wire also carries a `phase` (`commentary` vs
/// `final_answer`) that the canonical `text` block has nowhere to put, so it
/// stays unparsed on the wire.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentMessageItem {
    pub(crate) id: String,
    pub(crate) text: String,
}

/// A written plan. Not the running plan tracker — that arrives as
/// [`TurnPlanUpdatedNotification`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlanItem {
    pub(crate) id: String,
}

/// Model reasoning. Both fields default to empty, and an item carrying neither
/// is normal when reasoning summaries are off.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningItem {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) summary: Vec<String>,
    #[serde(default)]
    pub(crate) content: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandExecutionItem {
    pub(crate) id: String,
    pub(crate) command: String,
    pub(crate) cwd: String,
    pub(crate) status: CommandExecutionStatus,
    #[serde(default)]
    pub(crate) aggregated_output: Option<String>,
    #[serde(default)]
    pub(crate) exit_code: Option<i32>,
    #[serde(default)]
    pub(crate) duration_ms: Option<i64>,
    /// Which subsystem ran the command. Even a command the model asked for
    /// arrives as `unifiedExecStartup`, so a non-`agent` source must not be
    /// treated as somebody else's command and hidden.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the fixture test that pins the observed value; `codex.shell` has no slot for it"
        )
    )]
    #[serde(default)]
    pub(crate) source: Option<CommandExecutionSource>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CommandExecutionStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CommandExecutionSource {
    Agent,
    UserShell,
    UnifiedExecStartup,
    UnifiedExecInteraction,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileChangeItem {
    pub(crate) id: String,
    pub(crate) status: PatchApplyStatus,
    #[serde(default)]
    pub(crate) changes: Vec<FileUpdateChange>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PatchApplyStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileUpdateChange {
    pub(crate) path: String,
    pub(crate) kind: PatchChangeKind,
    pub(crate) diff: String,
}

/// How a file was changed. Internally tagged on `type`, unlike the exec
/// transport's bare `kind: "add"` string.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum PatchChangeKind {
    Add,
    Delete,
    Update {
        #[serde(default)]
        move_path: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpToolCallItem {
    pub(crate) id: String,
    pub(crate) server: String,
    pub(crate) tool: String,
    pub(crate) status: McpToolCallStatus,
    #[serde(default)]
    pub(crate) arguments: serde_json::Value,
    #[serde(default)]
    pub(crate) result: Option<McpToolCallResult>,
    #[serde(default)]
    pub(crate) error: Option<McpToolCallError>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum McpToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpToolCallResult {
    #[serde(default)]
    pub(crate) content: Vec<serde_json::Value>,
    #[serde(default)]
    pub(crate) structured_content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpToolCallError {
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DynamicToolCallItem {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) status: McpToolCallStatus,
    #[serde(default)]
    pub(crate) arguments: serde_json::Value,
    #[serde(default)]
    pub(crate) namespace: Option<String>,
    #[serde(default)]
    pub(crate) success: Option<bool>,
    #[serde(default)]
    pub(crate) content_items: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollabAgentToolCallItem {
    pub(crate) id: String,
    pub(crate) tool: CollabAgentTool,
    pub(crate) sender_thread_id: String,
    pub(crate) status: CollabAgentToolCallStatus,
    #[serde(default)]
    pub(crate) receiver_thread_ids: Vec<String>,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) agents_states: HashMap<String, CollabAgentState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CollabAgentTool {
    SpawnAgent,
    SendInput,
    ResumeAgent,
    Wait,
    CloseAgent,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CollabAgentToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollabAgentState {
    pub(crate) status: CollabAgentStatus,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CollabAgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubAgentActivityItem {
    pub(crate) id: String,
}

/// A web search. `action` is provider-shaped and is not introspected, so it
/// stays raw JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebSearchItem {
    pub(crate) id: String,
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) action: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageViewItem {
    pub(crate) id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SleepItem {
    pub(crate) id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageGenerationItem {
    pub(crate) id: String,
}

/// Both `enteredReviewMode` and `exitedReviewMode`; the two carry the same
/// shape.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewModeItem {
    pub(crate) id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextCompactionItem {
    pub(crate) id: String,
}

// =============================================================================
// JSON-RPC framing
// =============================================================================

/// A JSON-RPC 2.0 request id. The client only ever mints numbers; the server
/// uses either for its own requests, so both are accepted when replying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RequestId {
    Number(i64),
    String(String),
}

/// The whole of a JSON-RPC error object; `JSONRPCErrorError.json` declares
/// exactly these three fields.
///
/// Every `turn/steer` rejection measured returned `-32600` with a different
/// sentence, so callers must branch on the fact of failure, never on `message`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct JsonRpcErrorInfo {
    pub(crate) code: i64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<serde_json::Value>,
}

impl JsonRpcErrorInfo {
    /// The unavailable provider named by Codex's structured configuration error.
    pub(crate) fn missing_model_provider(&self) -> Option<&str> {
        let data = self.data.as_ref()?;
        (data["type"] == "modelProviderNotConfigured")
            .then(|| data["modelProvider"].as_str())
            .flatten()
    }
}

impl std::fmt::Display for JsonRpcErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

/// One message read off the server's stdout. `jsonrpc` is not required: the
/// measured server omits it on notifications and adds a top-level
/// `emittedAtMs`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IncomingMessage {
    #[serde(default)]
    pub(crate) id: Option<RequestId>,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) params: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) result: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) error: Option<JsonRpcErrorInfo>,
}

//! Adapter, trigger, and binding domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use moatless_backend_core::scope::Scope;

/// Persisted `task_bindings.origin` value for bindings created from inbound
/// provider events. The stored value remains `ingress` for existing database
/// rows and API compatibility.
pub const ADAPTER_EVENT_BINDING_ORIGIN: &str = "ingress";
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";

// =============================================================================
// Provider contexts
// =============================================================================

/// Provider-facing route context.
///
/// This is synthesized from canonical routes, triggers, bindings, or scheduled
/// jobs before invoking provider-specific delivery/parsing code.
#[derive(Debug, Clone)]
pub struct LoopContext {
    pub id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub prompt_template: Option<String>,
    pub active: bool,
    pub allow_direct_send: bool,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub scope: Scope,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    /// Per-trigger inactivity TTL override (days). `None` means the reaper
    /// falls back to the platform-wide default (`TASK_INACTIVE_TIMEOUT_DAYS`).
    /// Flows into `tasks.inactive_ttl_days` at task creation time.
    pub inactive_ttl_days: Option<i32>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LoopContext {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

// =============================================================================
/// Provider-facing subject context.
///
/// This is synthesized from a task binding before invoking provider-specific
/// outbound and context-fetching code.
#[derive(Debug, Clone)]
pub struct BindingContext {
    pub binding_id: String,
    pub trigger_id: Option<String>,
    pub task_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Target architecture tables (DB)
// =============================================================================

/// Row from the `integration_connections` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AdapterConnection {
    pub id: String,
    pub provider: String,
    pub connection_kind: String,
    pub scope_value: String,
    pub credential_ref: Option<String>,
    pub webhook_secret: Option<String>,
    /// Provider-specific transport metadata (JSONB). Each provider stores its
    /// own flat bag of identifiers and config here so we don't need
    /// per-provider columns on this table.
    ///
    /// Conventions in use today:
    /// - `source`: `"env"` | `"oauth"` — where the credential came from.
    /// - `slack_workspace_id`: e.g. `"T0000000002"` — the Slack workspace ID.
    ///   Used to build `task_bindings.subject_key` for DM replies so that
    ///   inbound webhook events route back to the originating task.
    pub transport_config: serde_json::Value,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Row from the `trigger_rules` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct LoopSpawnConfig {
    pub id: String,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub inactive_ttl_days: Option<i32>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LoopSpawnConfig {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

/// Canonical adapter-event trigger view backed by triggers, sources, and rules.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct SubscriptionMatch {
    pub id: String,
    pub provider: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub routing_mode: RoutingMode,
    pub event_filter: serde_json::Value,
    pub repository_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub inactive_ttl_days: Option<i32>,
    pub allow_direct_send: bool,
    pub provider_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SubscriptionMatch {
    /// The user ID to attribute tasks to when the actual sender cannot be resolved.
    /// Falls back to `created_by` when `run_as_user_id` is not set.
    pub fn effective_user_id(&self) -> &str {
        self.run_as_user_id.as_deref().unwrap_or(&self.created_by)
    }
}

/// Row from the `task_bindings` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskBinding {
    pub id: String,
    pub task_id: String,
    pub provider: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub connection_id: String,
    pub trigger_id: Option<String>,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    /// Display string for the connected subject's external status (GitHub PR /
    /// Linear issue). `#[sqlx(default)]` so the many existing `SELECT`s that
    /// don't project these columns keep deserializing — only the
    /// task-connections query reads them. Written by webhook handlers and the
    /// live status service.
    #[sqlx(default)]
    pub external_status: Option<String>,
    /// Normalized status bucket (GitHub: open|merged|closed|draft; Linear:
    /// state.type).
    #[sqlx(default)]
    pub external_state_kind: Option<String>,
    /// Full status detail (title, url, state id/name, merged flag, …).
    #[sqlx(default)]
    pub external_status_raw: Option<serde_json::Value>,
    /// Latest PR review verdict (GitHub: approved | changes_requested |
    /// review_required | dismissed | commented). `#[sqlx(default)]` so queries
    /// that don't project it still deserialize; written by the
    /// `pull_request_review` webhook path. `None` until a review is recorded.
    #[sqlx(default)]
    pub external_review_decision: Option<String>,
    /// GitHub's coarse mergeable state (clean | dirty | blocked | behind |
    /// unstable | has_hooks | draft); `dirty` means merge conflicts. `None`
    /// until computed / not applicable. Written by the per-event merge-conflict
    /// check.
    #[sqlx(default)]
    pub external_mergeable_state: Option<String>,
    /// When [`Self::external_status`] was last refreshed; `None` when never
    /// persisted.
    #[sqlx(default)]
    pub external_status_updated_at: Option<DateTime<Utc>>,
    /// Whether this binding still routes inbound events. Flipped to `false`
    /// when the task closes (a closed task is terminal — see
    /// `PgTaskBindingRepository::deactivate_by_task`). Inbound routing skips
    /// inactive bindings; the admin Task Connections view still shows them,
    /// flagged inactive.
    ///
    /// `#[sqlx(default)]` makes this `false` when a query doesn't project the
    /// column — the fail-safe direction (a forgotten projection drops delivery
    /// rather than resurrecting a task). The shared `TASK_BINDING_SELECT` and
    /// `upsert` RETURNING both project it, so live paths see the real value.
    #[sqlx(default)]
    pub active: bool,
    /// When this binding was deactivated; `None` while active.
    #[sqlx(default)]
    pub deactivated_at: Option<DateTime<Utc>>,
}

/// Row from the `schedule_trigger_sources` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Schedule {
    pub id: String,
    pub loop_id: String,
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Domain types (internal, not serialized to API)
// =============================================================================

pub use moatless_api_schemas::enums::RoutingMode;

/// Inbound message produced by a provider-specific parser.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Provider type.
    pub provider: crate::adapters::AdapterKind,
    /// Source identifier (Slack channelId, email address).
    pub source_id: String,
    /// Thread ID if this is a reply (Slack thread_ts, Email In-Reply-To).
    pub thread_id: Option<String>,
    /// New thread ID for the message itself (Slack message ts, Email Message-ID).
    pub message_thread_id: String,
    /// Resolved user ID (system user).
    pub user_id: String,
    /// Message text content.
    pub text: String,
    /// File attachment IDs (already uploaded to S3).
    pub file_ids: Vec<String>,
    /// Provider-specific properties to store on thread.
    pub thread_properties: serde_json::Value,
    /// Provider-specific context.
    pub provider_context: serde_json::Value,
    /// External message ID for dedup (e.g., Slack `ts`, Telegram `message_id`).
    /// Must be unique per message — NOT the thread ID.
    pub external_event_id: Option<String>,
    /// Source task id when the inbound event was authored by a moatless task
    /// (e.g. the GitHub PR / Linear adapters parse this from the `Moatless task:`
    /// footer link `<frontend_url>/tasks/<id>` on the comment body). Routing uses
    /// this to skip delivery back to the authoring task while still fanning out
    /// to other bindings on the same subject.
    pub source_task_id: Option<String>,
}

/// Resolved file attachment for outbound adapter messages.
#[derive(Clone, Debug)]
pub struct OutboundFile {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Outbound reply to send back to an external target.
#[derive(Debug)]
pub struct OutboundReply {
    pub text: String,
    pub task_url: Option<String>,
    /// Footer suffix appended to comment bodies by the Linear/GitHub adapters
    /// (e.g. `"\n\n---\nMoatless task: [Task name](https://app/tasks/<id>)"`).
    /// Built by [`crate::adapters::footer::task_comment_footer`]. The footer URL
    /// doubles as the inbound echo-filter source. `None` for providers that
    /// don't carry a footer.
    pub footer: Option<String>,
    /// Preferred system user whose provider credentials should author this
    /// reply — the owning task's `owner_user_id`. Providers that resolve a
    /// per-user credential (currently GitHub) prefer this over binding/trigger
    /// identities so a human-owned task replies under the human, and a
    /// trigger-created automation task replies under its run-as bot. `None`
    /// when no task owner is in scope (non-reply sends) or for providers that
    /// don't use it.
    pub author_user_id: Option<String>,
    pub is_error: bool,
    pub files: Vec<OutboundFile>,
}

/// Info returned by `send_message` to create a new `BindingContext`.
#[derive(Debug)]
pub struct NewThreadInfo {
    /// Provider-specific thread/message ID (e.g., Slack message `ts`).
    pub external_thread_id: String,
    /// Provider-specific properties for the thread (e.g., `channel`, `thread_ts`).
    pub properties: serde_json::Value,
}

// =============================================================================
// Shared access-control types (used by Telegram, Slack, and future providers)
// =============================================================================

/// Access policy for an external source (DM, group, or Slack channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceAccessPolicy {
    /// Accept messages from any source.
    #[default]
    Open,
    /// Reject all messages.
    Disabled,
    /// Accept only from sources in the allowlist.
    Allowlist,
    /// Allowlist + prompt unknown users with a pairing code (DM only).
    Pairing,
}

/// When to trigger processing of an inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageTrigger {
    /// Only process messages that mention the bot.
    #[default]
    MentionOnly,
    /// Process all messages in the source.
    AllMessages,
}

/// Result of an access control check.
#[derive(Debug)]
pub enum AccessDecision {
    /// Message is allowed through.
    Allowed,
    /// Message is blocked (silently dropped).
    Blocked,
    /// A pairing code was sent to the user; message is not processed.
    PairingSent,
}

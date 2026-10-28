//! Memory domain and row types.

use chrono::{DateTime, Utc};

use moatless_types::memory::{MemoryOp, MemoryScope};

/// A `memory_entries` row as [`super::dao`] selects it, with the claim and
/// staleness already resolved against the clock.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryEntryRow {
    pub entry_id: String,
    pub scope_kind: String,
    pub scope_id: String,
    pub key: String,
    pub summary: String,
    pub body: String,
    pub data: Option<serde_json::Value>,
    pub tags: Vec<String>,
    pub citations: Vec<String>,
    pub version: i32,
    pub ttl_seconds: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub stale_after_seconds: Option<i64>,
    pub verified_at: Option<DateTime<Utc>>,
    /// Only set while the claim is unexpired and its Task is open.
    pub claimed_by_task_id: Option<String>,
    pub claim_expires_at: Option<DateTime<Utc>>,
    pub updated_by_task_id: Option<String>,
    pub created_by_task_id: Option<String>,
    pub created_by_loop_id: Option<String>,
    pub created_by_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub stale: bool,
}

/// A `memory_events` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryEventRow {
    pub op: String,
    pub version: i32,
    pub text: Option<String>,
    pub task_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub use crate::scope_reach::ScopeRef;

/// Who made a write, recorded on the entry and its event.
#[derive(Debug, Clone, Default)]
pub struct Writer {
    pub task_id: String,
    pub loop_id: Option<String>,
    pub user_id: String,
}

/// A validated put. `None` keeps the entry's current value.
#[derive(Debug, Clone)]
pub struct PutEntry {
    pub scope: ScopeRef,
    pub key: String,
    pub summary: Option<String>,
    pub body: Option<String>,
    pub data: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub citations: Option<Vec<String>>,
    pub ttl: TtlChange,
    pub stale_after_seconds: Option<i64>,
    pub if_version: Option<i32>,
    pub writer: Writer,
}

/// How a put changes an entry's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtlChange {
    /// The entry's current TTL, or the default for a new entry.
    Keep,
    Set(i64),
    NoExpiry,
}

/// What became of a put.
#[derive(Debug, Clone)]
pub enum PutOutcome {
    Written(Box<MemoryEntryRow>),
    /// `if_version` did not match; `current` is `None` when there is no entry.
    VersionMismatch {
        current: Option<i32>,
    },
    /// The key is new and the put carried no summary.
    SummaryRequired,
}

/// What became of a claim.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    Claimed(Box<MemoryEntryRow>),
    HeldBy(String),
}

/// Narrowing for a listing.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub prefix: Option<String>,
    pub tag: Option<String>,
    pub updated_since: Option<DateTime<Utc>>,
    pub limit: i64,
}

/// Narrows a preload to entries a Task other than `reader_task_id` wrote
/// after `since`.
#[derive(Debug, Clone)]
pub struct ChangedSince {
    pub since: DateTime<Utc>,
    pub reader_task_id: String,
}

/// A task's `memory_seen` read, timed by the database clock that stamps
/// `updated_at`.
#[derive(Debug, Clone, Copy)]
pub struct SeenAt {
    /// The database's `now()` when this was read.
    pub read_at: DateTime<Utc>,
    /// The committed time; `None` before the task's first commit.
    pub seen_at: Option<DateTime<Utc>>,
}

/// A row's `scope_kind`, decoded; `None` for a value the migration's CHECK
/// would have refused.
pub(crate) fn decode_scope(row: &MemoryEntryRow) -> Option<MemoryScope> {
    MemoryScope::try_from(row.scope_kind.as_str()).ok()
}

/// A row's `op`, decoded.
pub(crate) fn decode_op(row: &MemoryEventRow) -> Option<MemoryOp> {
    MemoryOp::try_from(row.op.as_str()).ok()
}

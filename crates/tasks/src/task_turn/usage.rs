//! Token and cost totals summed from the metrics each Turn recorded.

use moatless_core::db::DbPool;
use moatless_types::access_control::ScopeContext;

/// The days a usage read covers, already validated.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub since_day: String,
    pub until_day: String,
    /// IANA zone the days are bucketed in. An offset would be wrong for any
    /// window crossing a DST boundary.
    pub time_zone: String,
    /// Bucket by hour instead of by day.
    pub hourly: bool,
}

/// One `(day, provider, model)` cell of usage, already aggregated.
///
/// Token fields are disjoint: `uncached_input_tokens`, `cached_input_tokens`
/// and `cache_creation_tokens` sum to the total input; `reasoning_tokens` is a
/// subset of `output_tokens`, never added on top.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBucket {
    /// `YYYY-MM-DD` in the window's time zone.
    pub day: String,
    /// RFC3339 start of the hour, present only for an hourly window.
    pub hour_start: Option<String>,
    /// `claude` or `codex`, from the Task's agent profile.
    pub provider: String,
    pub model: String,
    pub uncached_input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    /// What the provider itself reported for these tokens, summed.
    pub cost_usd: f64,
    /// Turns that contributed.
    pub records: i64,
    /// Turns whose tokens are counted and whose cost the provider never
    /// reported.
    pub unpriced_records: i64,
    /// Distinct Tasks.
    pub sessions: i64,
}

/// Per-model usage over the Turns a viewer may read.
///
/// The agent runs in a Sandbox pod that gets deleted, so the totals it
/// reported, kept in `task_turns.metrics`, are the only lasting record.
pub struct TurnUsage {
    pool: DbPool,
}

impl TurnUsage {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// The buckets for `window`, ordered by day, provider and model.
    pub async fn buckets(
        &self,
        viewer: &ScopeContext,
        window: &UsageWindow,
    ) -> Result<Vec<UsageBucket>, sqlx::Error> {
        super::usage_dao::buckets(&self.pool, viewer, window).await
    }
}

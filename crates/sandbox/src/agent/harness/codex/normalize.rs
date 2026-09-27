//! Native Codex thread item → canonical content block normalization.
//!
//! One canonical block (or paired `tool_use`/`tool_result`) per native item,
//! in native order, so every item family keeps its observable meaning without
//! the harness re-deriving native payloads. This is the only place that maps
//! Codex-native items; the shared harness/driver never sees a [`ThreadItem`].

use crate::agent::harness::content::CanonicalContentBlock;

use crate::agent::codex_app_server::protocol::{ThreadItem, TokenUsageBreakdown, TurnPlanStep};

use super::super::signal::{RunTotals, SessionId};
use super::blocks;

/// Collision-safe Codex normalizer. Holds the last-known Session (the Codex
/// thread), the requested model, and a per-spawn nonce so a `tool_use.id`
/// stays unique even when the CLI reuses `item_0` after a resume.
pub(super) struct Normalizer {
    session: Option<SessionId>,
    model: Option<String>,
    /// Per-spawn collision-avoidance element mixed into every emitted id.
    nonce: String,
}

impl Normalizer {
    pub(super) fn new(model: Option<String>) -> Self {
        Self {
            session: None,
            model,
            nonce: uuid::Uuid::new_v4().simple().to_string()[..8].to_string(),
        }
    }

    /// Record the Codex thread as the Session and refresh the per-spawn nonce.
    /// A fresh nonce on every `thread.started` keeps ids disjoint across the
    /// resume-driven `item_0` restart.
    pub(super) fn set_session(&mut self, session: SessionId) {
        self.session = Some(session);
        self.nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    }

    fn session_or(&self, native: &str) -> SessionId {
        self.session
            .clone()
            .unwrap_or_else(|| SessionId(native.to_string()))
    }

    /// Qualify a native id with the Session and per-spawn nonce so it survives
    /// Codex's per-resume `item_0` restart without colliding. App-server ids
    /// look globally unique, but that isn't established for every item
    /// family, so the `{session}_{nonce}_{item}` shape is what persisted
    /// reads line up against.
    fn qualify(&self, session: &SessionId, native: &str) -> String {
        format!("{}_{}_{native}", session.0, self.nonce)
    }

    /// Canonical blocks for one native Codex item, in native order (see
    /// [`blocks::item_blocks`] for the unknown/dropped-family cases).
    pub(super) fn item_sources(&self, item: &ThreadItem) -> Vec<CanonicalContentBlock> {
        let session = self.session_or(item.id());
        let qualified = self.qualify(&session, item.id());

        match blocks::item_blocks(item, &qualified) {
            Some(blocks) => blocks,
            None => vec![unknown_block(item)],
        }
    }

    /// Canonical blocks for the running plan tracker, which arrives as a
    /// notification and so has no item id of its own.
    pub(super) fn plan_sources(&self, plan: &[TurnPlanStep]) -> Vec<CanonicalContentBlock> {
        let session = self.session_or("");
        let qualified = self.qualify(&session, "plan");
        blocks::plan_blocks(plan, &qualified)
    }

    /// The run totals for a completed Codex Turn.
    ///
    /// `input_tokens` is normalized to non-cached input (the Codex breakdown's
    /// `input_tokens` is gross and double-counts the `cached_input_tokens`
    /// prefix) so the backend prices every harness the same way;
    /// `total_cost_usd` is always `None` since Codex reports no cost.
    pub(super) fn run_totals(&self, usage: &TurnUsage) -> RunTotals {
        RunTotals {
            input_tokens: (usage.input_tokens - usage.cached_input_tokens).max(0),
            output_tokens: usage.output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: usage.cached_input_tokens,
            reasoning_output_tokens: usage.reasoning_output_tokens,
            total_cost_usd: None,
            num_turns: None,
            duration_ms: None,
            duration_api_ms: None,
            model: self.model.clone(),
            model_usage: None,
        }
    }
}

/// Canonical block for a completed turn, carrying no usage of its own — the
/// turn's spend rides the terminal outcome via [`Normalizer::run_totals`]
/// instead.
pub(super) fn turn_result_source() -> CanonicalContentBlock {
    CanonicalContentBlock::TurnResult
}

/// Preserve an unknown family's raw native block so an upstream addition is
/// visible rather than discarded.
fn unknown_block(item: &ThreadItem) -> CanonicalContentBlock {
    // `item_blocks` returns `None` only for `Other`, which carries its
    // payload; anything else would be a mapping bug, not lost data.
    let raw = match item {
        ThreadItem::Other { raw, .. } => raw.clone(),
        other => serde_json::json!({ "type": other.type_name() }),
    };
    CanonicalContentBlock::Unknown { raw }
}

/// What one Turn spent, in the four fields the turn-result block publishes.
///
/// Assembled from `thread/tokenUsage/updated`, which reports two running
/// figures, neither of which is a Turn: `last` is the most recent model
/// request, `total` is everything the Session has spent, carried across
/// `thread/resume` (see [`TurnUsage::between`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "the field names are the wire's; dropping the postfix would need a rename on every one"
)]
pub(super) struct TurnUsage {
    pub(super) input_tokens: i64,
    pub(super) cached_input_tokens: i64,
    pub(super) output_tokens: i64,
    pub(super) reasoning_output_tokens: i64,
}

impl TurnUsage {
    /// The spend between two `total` readings. `baseline` is what the Session
    /// had already spent when the Turn opened — `None` on a fresh thread, where
    /// the first reading is the Turn's own first request.
    pub(super) fn between(
        baseline: Option<&TokenUsageBreakdown>,
        latest: &TokenUsageBreakdown,
    ) -> Self {
        let zero = TokenUsageBreakdown::default();
        let base = baseline.unwrap_or(&zero);
        Self {
            input_tokens: (latest.input_tokens - base.input_tokens).max(0),
            cached_input_tokens: (latest.cached_input_tokens - base.cached_input_tokens).max(0),
            output_tokens: (latest.output_tokens - base.output_tokens).max(0),
            reasoning_output_tokens: (latest.reasoning_output_tokens
                - base.reasoning_output_tokens)
                .max(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breakdown(input: i64, cached: i64, output: i64, reasoning: i64) -> TokenUsageBreakdown {
        TokenUsageBreakdown {
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: output,
            reasoning_output_tokens: reasoning,
        }
    }

    /// A Turn's spend is the *growth* of the cumulative `total` while it ran,
    /// not the cumulative figure itself.
    ///
    /// Guards against the bug that leaked session-cumulative counts as per-turn
    /// usage: were `between` to return the latest reading, a second Turn would
    /// report the whole session's tokens and cost every time.
    #[test]
    fn between_is_the_growth_of_the_cumulative_total_not_its_absolute() {
        // codex-cli 0.145.0: a three-request Turn whose cumulative `total` ran
        // 15842 → 47729 spent 31887, opening from a session already at 15842.
        let baseline = breakdown(15842, 0, 0, 0);
        let latest = breakdown(47729, 0, 0, 0);
        let usage = TurnUsage::between(Some(&baseline), &latest);
        assert_eq!(usage.input_tokens, 31887);

        // A fresh thread has no baseline: the first reading is the Turn's own.
        let first = TurnUsage::between(None, &breakdown(15842, 0, 0, 0));
        assert_eq!(first.input_tokens, 15842);
    }

    /// `run_totals` normalizes input to the non-cached tokens and carries the
    /// cached-prefix reads as their own tier — the tier that was silently
    /// dropped at the old `(input, output)` seam.
    #[test]
    fn run_totals_splits_net_input_from_cache_read() {
        let n = Normalizer::new(Some("gpt-5-codex".to_string()));
        let usage = TurnUsage {
            input_tokens: 1000,
            cached_input_tokens: 400,
            output_tokens: 200,
            reasoning_output_tokens: 50,
        };
        let totals = n.run_totals(&usage);
        assert_eq!(totals.input_tokens, 600);
        assert_eq!(totals.cache_read_input_tokens, 400);
        assert_eq!(totals.cache_creation_input_tokens, 0);
        assert_eq!(totals.output_tokens, 200);
        assert_eq!(totals.reasoning_output_tokens, 50);
        // Codex reports no cost; the backend prices from `model` and the tokens.
        assert_eq!(totals.total_cost_usd, None);
        assert_eq!(totals.model.as_deref(), Some("gpt-5-codex"));
    }
}

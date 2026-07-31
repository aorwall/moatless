//! Native Codex thread item → canonical `SourceEvent` normalization.
//!
//! One `SourceEvent` per native item, in native order, so every item family
//! keeps its observable meaning without the harness re-deriving native payloads.
//! This is the only place that maps Codex-native items; the shared
//! harness/driver never sees a [`ThreadItem`].
//!
//! A per-spawn nonce is mixed into each `source_id`. It was introduced because
//! `codex exec --resume` restarted item ids at `item_0`; app-server ids look
//! globally unique, but that was not established for every family and the
//! `{session}_{nonce}_{item}` scheme is what persisted reads line up against, so
//! it stays.

use moatless_api_schemas::content::CanonicalContentBlock;

use crate::agent::codex_app_server::protocol::{ThreadItem, TokenUsageBreakdown, TurnPlanStep};

use super::super::signal::SessionId;
use super::super::source::{NativeMessageId, Role, SourceEvent, SourceId};
use super::blocks;
use super::blocks::ItemBlock;

/// Upper bound on retained native diagnostics for an unknown item block.
const MAX_DIAGNOSTIC_BYTES: usize = 2000;

/// Collision-safe Codex normalizer. Holds the last-known Session (the Codex
/// thread), the requested model, and a per-spawn nonce so `source_id`s stay
/// unique even when the CLI reuses `item_0` after a resume.
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
    /// Codex's per-resume `item_0` restart without colliding. Mirrors the legacy
    /// `{session}_{nonce}_{item}` UUID scheme so persisted reads line up.
    fn qualify(&self, session: &SessionId, native: &str) -> String {
        format!("{}_{}_{native}", session.0, self.nonce)
    }

    /// Canonical source events for one native Codex item, in native order. Each
    /// native family is mapped to the `text` / `tool_use` / `tool_result` blocks
    /// the backend assembler and UI understand (a tool item expands to a
    /// tool_use plus a paired tool_result in its own user Message). An unknown
    /// family is forwarded as its raw native block with bounded diagnostics so an
    /// upstream CLI addition is never silently discarded; a family that is known
    /// and deliberately renders nothing yields no events at all.
    pub(super) fn item_sources(&self, item: &ThreadItem) -> Vec<SourceEvent> {
        let session = self.session_or(item.id());
        let qualified = self.qualify(&session, item.id());

        match blocks::item_blocks(item, &qualified) {
            Some(blocks) => self.events(blocks, &session, &qualified),
            None => vec![self.unknown_source(item, session, qualified)],
        }
    }

    /// Preserve an unknown family's raw native block, flagged with bounded
    /// diagnostics, so an upstream addition is visible rather than discarded.
    fn unknown_source(
        &self,
        item: &ThreadItem,
        session: SessionId,
        qualified: String,
    ) -> SourceEvent {
        // `item_blocks` returns `None` only for `Other`, which carries its
        // payload; anything else would be a mapping bug, not lost data.
        let raw = match item {
            ThreadItem::Other { raw, .. } => raw.clone(),
            other => serde_json::json!({ "type": other.type_name() }),
        };
        SourceEvent {
            source_id: SourceId(qualified.clone()),
            message_id: NativeMessageId(qualified),
            session,
            parent_tool_use_id: None,
            role: Role::Assistant,
            block: CanonicalContentBlock::Unknown { raw: raw.clone() },
            tool_use_result: None,
            usage: None,
            model: self.model.clone(),
            diagnostics: Some(bounded(&raw)),
        }
    }

    /// Canonical source events for the running plan tracker, which arrives as a
    /// notification and so has no item id of its own. One stable id per spawn
    /// keeps every update upserting the same live card, as the `todo_list`
    /// item's re-used id did.
    pub(super) fn plan_sources(&self, plan: &[TurnPlanStep]) -> Vec<SourceEvent> {
        let session = self.session_or("");
        let qualified = self.qualify(&session, "plan");
        self.events(blocks::plan_blocks(plan, &qualified), &session, &qualified)
    }

    fn events(
        &self,
        blocks: Vec<ItemBlock>,
        session: &SessionId,
        qualified: &str,
    ) -> Vec<SourceEvent> {
        blocks
            .into_iter()
            .map(|canonical| {
                // A tool_result lands in its own Message (`{id}_result`) so it is
                // grouped and role-assigned separately from the tool_use.
                let id = format!("{qualified}{}", canonical.id_suffix);
                SourceEvent {
                    source_id: SourceId(id.clone()),
                    message_id: NativeMessageId(id),
                    session: session.clone(),
                    // Codex items carry no per-item parent tool attribution; collab
                    // relationships ride inside the item block itself.
                    parent_tool_use_id: None,
                    role: canonical.role,
                    block: canonical.block,
                    tool_use_result: canonical.tool_use_result,
                    // Codex reports usage per turn, not per item — carried on the
                    // turn-result source event instead.
                    usage: None,
                    model: self.model.clone(),
                    diagnostics: None,
                }
            })
            .collect()
    }

    /// Canonical source event for a completed turn, carrying the turn-level usage
    /// so it is retained through the seam rather than dropped at the terminal,
    /// which has no usage carrier.
    pub(super) fn turn_result_source(&self, usage: &TurnUsage) -> SourceEvent {
        let session = self.session_or("");
        let id = format!("{}_{}_result", session.0, self.nonce);
        SourceEvent {
            source_id: SourceId(id.clone()),
            message_id: NativeMessageId(id),
            session,
            parent_tool_use_id: None,
            role: Role::System,
            block: CanonicalContentBlock::TurnResult,
            tool_use_result: None,
            usage: Some(serde_json::json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cached_input_tokens,
                "reasoning_output_tokens": usage.reasoning_output_tokens,
            })),
            model: self.model.clone(),
            diagnostics: None,
        }
    }
}

/// What one Turn spent, in the four fields the turn-result block publishes.
///
/// Assembled from `thread/tokenUsage/updated`, which reports two running
/// figures and neither of them is a Turn: `last` is the most recent *model
/// request* and `total` is everything the Session has spent, carried across
/// `thread/resume`. Measured against codex-cli 0.145.0 — a three-request Turn
/// reported `last` totals of 15842/15918/15969 against `total` 15842/31760/47729,
/// and `codex exec --json` billed the same work as one `usage` of 47902 input
/// tokens. So the Turn's spend is the *growth* of `total` while it ran, which is
/// what [`TurnUsage::between`] computes.
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

/// Bound a native diagnostic value to a fixed byte budget so an oversized
/// unknown item cannot blow up a retained diagnostic string.
fn bounded(raw: &serde_json::Value) -> serde_json::Value {
    let text = raw.to_string();
    if text.len() <= MAX_DIAGNOSTIC_BYTES {
        return raw.clone();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::Value::String(format!("{}…", &text[..end]))
}

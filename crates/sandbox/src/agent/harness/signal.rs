//! The command/signal vocabulary at the external harness seam.
//!
//! The coordinator submits [`Input`], cancels the active execution, and
//! consumes an ordered [`Signal`] stream. Three orderings: opening a new
//! execution is `ExecutionStarted → InputAccepted → Source* →
//! ExecutionFinished`; joining an active one is `InputAccepted →
//! (Source*/Activity*)*`, with no new `ExecutionStarted` (`ExecutionFinished`
//! may arrive first); a harness-internal execution is `ExecutionStarted
//! (internal) → Source* → ExecutionFinished`.

use std::collections::HashMap;

use crate::agent::types::{ForkSource, ResumeSession};

use super::content::CanonicalContentBlock;
use super::error::HarnessError;

/// Correlation identity for a submitted input (a pending-Message batch). The
/// coordinator uses it to acknowledge the batch once it is accepted or
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputId(pub String);

/// Resumable CLI conversation identity, owned by the harness and reported to
/// the coordinator on [`Signal::ExecutionStarted`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Input submitted to the harness for one Turn.
///
/// Turn configuration (model, mode, resume, fresh start) never selects the
/// harness implementation, fixed at construction; `env` and `system_prompt`
/// are resolved by the coordinator and can change between Turns, so they
/// ride on each Input rather than being baked into the harness config.
#[derive(Debug, Clone)]
pub struct Input {
    pub id: InputId,
    pub text: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    /// Reasoning effort for this Turn, in the agent's own vocabulary. `None`
    /// leaves the agent on its own default.
    pub effort: Option<String>,
    pub resume: Option<ResumeSession>,
    /// Begin a new Session instead of resuming or injecting into a running one.
    pub start_fresh: bool,
    /// Branch the named Session at the named cut and run this Input on the
    /// branch. Set only on a fork's first turn, and on any retry of it.
    ///
    /// A harness must test this *before* `start_fresh` (the version-skew
    /// guard): an older pod ignores the field and takes a plain fresh start
    /// instead of resuming into the parent's transcript.
    pub fork: Option<ForkSource>,
    /// The session this Input continues is the other harness's: open the
    /// Task's switch session (or, with `fork`, the fork's) rendered from the
    /// Trajectory, and fail the Turn when it cannot be written. Tested after
    /// `fork` and before `start_fresh`, which is set alongside it.
    pub switch_harness: bool,
    /// Resolved subprocess environment for this Turn (credentials the
    /// coordinator refreshed before submit).
    pub env: Option<HashMap<String, String>>,
    /// Resolved append-system-prompt for this Turn.
    pub system_prompt: Option<String>,
}

impl Input {
    /// Build a plain text input with no Turn configuration overrides.
    pub fn text(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: InputId(id.into()),
            text: text.into(),
            model: None,
            mode: None,
            effort: None,
            resume: None,
            start_fresh: false,
            fork: None,
            switch_harness: false,
            env: None,
            system_prompt: None,
        }
    }
}

/// Whether `text` opens with a slash command such as `/compact` or
/// `/plugin:skill`, which the CLI runs only when it leads the prompt. A path
/// like `/opt/app is down` is no command.
pub fn is_slash_command(text: &str) -> bool {
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        return false;
    };
    let name = rest.split(char::is_whitespace).next().unwrap_or_default();
    name.starts_with(|c: char| c.is_ascii_alphabetic()) && !name.contains('/')
}

/// Why an execution started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// Opened by submitted input; carries the correlation identity.
    Input(InputId),
    /// Started by the harness itself (a wakeup or Monitor), with no pending
    /// Message behind it — so no [`Signal::InputAccepted`] follows.
    Internal,
}

/// The single terminal outcome of a started execution.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The execution completed normally.
    ///
    /// `totals` carries the run's authoritative token/cost figures when the
    /// harness reported them on its terminal, and is `None` for a completion
    /// that carried none (a turn that ended without a result message).
    Completed { totals: Option<RunTotals> },
    /// The execution was cancelled.
    Cancelled,
    /// The execution failed with a classified error.
    Failed(HarnessError),
}

/// A completed run's token/cost figures, as the harness reported them on
/// its terminal result.
///
/// Per-run deltas, not session-cumulative; the backend sums across a
/// task's turns and must never re-derive a cumulative total. `total_cost_usd`
/// is `Some` only for Claude; Codex leaves it `None`, priced from `model`.
#[derive(Debug, Clone, PartialEq)]
pub struct RunTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    /// Codex reasoning tokens; `0` for Claude, which has no separate count.
    pub reasoning_output_tokens: i64,
    pub total_cost_usd: Option<f64>,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
    pub duration_api_ms: Option<u64>,
    pub model: Option<String>,
    /// Claude's per-model `modelUsage` map, passed through verbatim for the
    /// backend's per-model cost breakdown. `None` for Codex.
    pub model_usage: Option<serde_json::Value>,
}

impl RunTotals {
    /// The `agent.completed` payload the backend parses: the run's cost, the
    /// four-plus-one token counts under `usage`, and the passthrough fields.
    /// Uses `snake_case` keys, which the backend accepts alongside camelCase.
    pub fn to_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "total_cost_usd": self.total_cost_usd,
            "usage": {
                "input_tokens": self.input_tokens,
                "output_tokens": self.output_tokens,
                "cache_creation_input_tokens": self.cache_creation_input_tokens,
                "cache_read_input_tokens": self.cache_read_input_tokens,
                "reasoning_output_tokens": self.reasoning_output_tokens,
            },
            "modelUsage": self.model_usage,
            "num_turns": self.num_turns,
            "duration_ms": self.duration_ms,
            "duration_api_ms": self.duration_api_ms,
            "model": self.model,
        })
    }
}

/// Why a submitted input never opened an execution.
///
/// A pre-start rejection is coordinator-safe: because no
/// [`Signal::ExecutionStarted`] was emitted, the coordinator never minted a
/// Run, so there is no Run to close.
#[derive(Debug, Clone, PartialEq)]
pub enum Rejection {
    /// The input was cancelled before its execution started.
    Cancelled,
    /// The input failed before its execution started, with a classified error.
    Failed(HarnessError),
}

/// Liveness / tool-progress activity. Keeps an active Turn visibly alive
/// without becoming Message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub session: SessionId,
    pub detail: String,
}

/// What the harness reported about its own context window.
///
/// Normalized so neither backend nor client learns the wire difference
/// (Claude's per-model `contextWindow` vs Codex's `modelContextWindow`);
/// `max_tokens` is `None` where the harness reported none, and the meter
/// then shows a count rather than a percentage of a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextWindow {
    pub used_tokens: i64,
    pub max_tokens: Option<i64>,
}

/// A normalized signal emitted by the harness. This is the entire external
/// output vocabulary; the coordinator reacts only to these.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// An Agent process is ready and its Session identity is known; the
    /// coordinator mints the Run when it receives this.
    ///
    /// Emitted at most once per execution, never after a source event;
    /// `agent_turn_id` is the harness's native turn id when known at start
    /// (Codex only), relayed so a fork can cut the thread there.
    ExecutionStarted {
        session: SessionId,
        agent_turn_id: Option<String>,
        cause: Cause,
    },
    /// The Agent process accepted a submitted input. The coordinator
    /// acknowledges opening inputs after this, against the current Run. For
    /// active-execution injections, it waits for later non-terminal output.
    InputAccepted { input: InputId },
    /// One native content block, in native order.
    Source(CanonicalContentBlock),
    /// Liveness / tool progress; never Message content.
    Activity(Activity),
    /// What the harness reported about its own context window.
    ContextWindow(ContextWindow),
    /// The single terminal outcome of a started execution. Exactly one is
    /// emitted per [`Signal::ExecutionStarted`].
    ExecutionFinished { outcome: Outcome },
    /// A submitted input that never opened an execution.
    InputRejected { input: InputId, reason: Rejection },
}

#[cfg(test)]
mod tests {
    use super::is_slash_command;

    #[test]
    fn slash_command_needs_a_leading_command_name() {
        for text in ["/compact", "  /compact keep the plan", "/general:show-me x"] {
            assert!(is_slash_command(text), "{text:?}");
        }
        for text in [
            "",
            "compact",
            "/",
            "/opt/app is down",
            "/ space",
            "/1x",
            "see /compact",
        ] {
            assert!(!is_slash_command(text), "{text:?}");
        }
    }
}

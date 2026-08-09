//! Prometheus metrics for agent runs and tool calls.
//!
//! These are emitted from the event handler as it processes the agent event
//! stream (`agent.message`, `agent.completed`, `agent.error`, `agent.stopped`).
//! They surface what the agent is actually *doing* inside a sandbox — tool
//! usage, failures, turn latency, cost — which the K8s-derived fleet metrics
//! (deployments, restarts, terminations) can't see.
//!
//! ## Cardinality
//! Labels are deliberately bounded:
//! - `tool` is bucketed via [`bucket_tool`] — built-in tools keep their name,
//!   every `mcp__*` tool collapses to `mcp`, everything else to `other`. This
//!   stops open-ended MCP / skill names from exploding the series count.
//! - `agent_type` is the handful of agent runtimes (claude, codex, …); `unknown`
//!   when absent.
//! - `status` ∈ {`ok`, `error`}; `outcome` ∈ a small fixed terminal-state set.
//! - `context_bucket` is the five fixed size classes of [`context_bucket`], plus
//!   `unknown`; `kind` is the four token classes the SDK usage object reports.

use super::models::UsageInfo;

/// Per-tool-call counter, labelled by bucketed `tool`, `agent_type`, and
/// `status` (`ok` | `error`). Emitted once per `tool_result` block, so it
/// counts *completed* tool calls by outcome.
pub(crate) const AGENT_TOOL_CALLS_TOTAL: &str = "moatless_agent_tool_calls_total";

/// Terminal agent-run outcomes (`completed` | `error` | `stopped`) by
/// `agent_type`. One increment per terminal agent event.
pub(crate) const AGENT_RUNS_TOTAL: &str = "moatless_agent_runs_total";

/// Agent cost in USD per run by `agent_type` (histogram). A histogram (not a
/// counter) because cost is fractional and the `metrics` counter only takes
/// `u64`; the histogram's `_sum` accumulates real-USD spend (rate of `_sum`
/// gives $/s) while the buckets show per-run cost distribution.
pub(crate) const AGENT_COST_USD: &str = "moatless_agent_cost_usd";

/// Wall-clock turn duration in seconds by `agent_type`, `outcome`, and
/// `context_bucket` (histogram). Sourced from the SDK result's `duration_ms`.
/// The `context_bucket` label is what makes "a big context makes a turn slow"
/// visible: without it the histogram mixes a 20k-context turn and a 400k one.
pub(crate) const AGENT_TURN_DURATION_SECONDS: &str = "moatless_agent_turn_duration_seconds";

/// Tokens consumed by a run, by `agent_type` and `kind` (`input` | `output` |
/// `cache_read` | `cache_creation`) — counter. Sourced from the SDK result's
/// `usage`, which reports the run's authoritative totals; the per-message
/// `output_tokens` on the stream is a partial value sampled before the response
/// finished, so it is not summed here.
pub(crate) const AGENT_TOKENS_TOTAL: &str = "moatless_agent_tokens_total";

/// API-time portion of a turn in seconds by `agent_type` (histogram). Sourced
/// from the SDK result's `duration_api_ms` (the tmux backend reports 0 — those
/// are dropped, see call site).
pub(crate) const AGENT_TURN_API_DURATION_SECONDS: &str = "moatless_agent_turn_api_duration_seconds";

/// Number of internal turns per agent run by `agent_type` (histogram). Sourced
/// from the SDK result's `num_turns`.
pub(crate) const AGENT_TURNS: &str = "moatless_agent_turns";

/// Tool-permission denials by bucketed `tool` (counter). Sourced from the SDK
/// result's `permission_denials`.
pub(crate) const AGENT_PERMISSION_DENIALS_TOTAL: &str = "moatless_agent_permission_denials_total";

/// Built-in Claude Code tool names that are safe to keep as a metric label
/// (bounded vocabulary). Anything not here is bucketed (see [`bucket_tool`]).
const KNOWN_TOOLS: &[&str] = &[
    "Bash",
    "BashOutput",
    "KillShell",
    "Read",
    "Edit",
    "MultiEdit",
    "Write",
    "NotebookEdit",
    "NotebookRead",
    "Glob",
    "Grep",
    "LS",
    "Task",
    "Agent",
    "WebFetch",
    "WebSearch",
    "TodoWrite",
    "Skill",
    "SlashCommand",
    "AskUserQuestion",
    "ExitPlanMode",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];

/// Bucket a raw tool name into a bounded, low-cardinality label.
///
/// Built-in tools keep their own name; `mcp__server__tool` names collapse to
/// `mcp`; anything else (unknown / dynamically named) collapses to `other`.
/// Returns a `&'static str` so it can be used directly as a metric label and
/// cached cheaply.
pub(crate) fn bucket_tool(name: &str) -> &'static str {
    if let Some(known) = KNOWN_TOOLS.iter().find(|&&t| t == name) {
        return known;
    }
    if name.starts_with("mcp__") {
        return "mcp";
    }
    "other"
}

/// Bucket the prompt size one of a run's internal turns carried, as a bounded
/// label.
///
/// The SDK result reports usage for the whole run — the sum over its internal
/// API calls — so the input side is divided by `num_turns` to recover what a
/// single call actually carried. That per-call size is what drives
/// time-to-first-token, and the run total obscures it: a 20-turn run at a 100k
/// context and a 4-turn run at a 500k one both report 2M input tokens.
///
/// Only the input side counts (prompt + cache read + cache creation); output
/// tokens are produced by the call, not fed to it.
///
/// Label values are zero-padded lower bounds so the lexical order Prometheus and
/// Grafana sort by is also the numeric one.
pub(crate) fn context_bucket(usage: Option<&UsageInfo>, num_turns: Option<u32>) -> &'static str {
    let Some(usage) = usage else {
        return "unknown";
    };
    let input = usage.input_tokens.unwrap_or(0)
        + usage.cache_read_input_tokens.unwrap_or(0)
        + usage.cache_creation_input_tokens.unwrap_or(0);
    if input <= 0 {
        return "unknown";
    }
    // A result reporting 0 turns still ran one call — never divide by zero.
    let turns = i64::from(num_turns.unwrap_or(1).max(1));
    let mean = input / turns;
    if mean < 50_000 {
        "000-050k"
    } else if mean < 100_000 {
        "050-100k"
    } else if mean < 200_000 {
        "100-200k"
    } else if mean < 400_000 {
        "200-400k"
    } else {
        "400k+"
    }
}

/// Normalise an optional agent-type label to a non-empty owned string.
fn agent_label(agent_type: Option<&str>) -> String {
    agent_type
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// Register metric descriptions with the Prometheus recorder.
///
/// Call once at startup after the recorder is installed.
pub fn describe_agent_metrics() {
    metrics::describe_counter!(
        AGENT_TOOL_CALLS_TOTAL,
        "Completed agent tool calls by bucketed tool, agent_type, and status (ok | error)"
    );
    metrics::describe_counter!(
        AGENT_RUNS_TOTAL,
        "Terminal agent-run outcomes (completed | error | stopped) by agent_type"
    );
    metrics::describe_histogram!(
        AGENT_COST_USD,
        "Agent cost in USD per run by agent_type (sum = total spend, buckets = per-run distribution)"
    );
    metrics::describe_histogram!(
        AGENT_TURN_DURATION_SECONDS,
        "Agent turn wall-clock duration in seconds by agent_type, outcome, and context_bucket (mean prompt size per internal turn)"
    );
    metrics::describe_counter!(
        AGENT_TOKENS_TOTAL,
        "Tokens consumed by agent runs by agent_type and kind (input | output | cache_read | cache_creation)"
    );
    metrics::describe_histogram!(
        AGENT_TURN_API_DURATION_SECONDS,
        "Agent turn API time in seconds by agent_type"
    );
    metrics::describe_histogram!(AGENT_TURNS, "Internal turns per agent run by agent_type");
    metrics::describe_counter!(
        AGENT_PERMISSION_DENIALS_TOTAL,
        "Tool-permission denials by bucketed tool"
    );
}

/// Record one completed tool call. `tool` must already be bucketed
/// ([`bucket_tool`]); `status` is `"ok"` or `"error"`.
pub(crate) fn record_tool_call(tool: &'static str, agent_type: Option<&str>, status: &'static str) {
    metrics::counter!(
        AGENT_TOOL_CALLS_TOTAL,
        "tool" => tool,
        "agent_type" => agent_label(agent_type),
        "status" => status,
    )
    .increment(1);
}

/// Record one terminal agent-run outcome (`"completed"`, `"error"`, `"stopped"`).
pub(crate) fn record_run(outcome: &'static str, agent_type: Option<&str>) {
    metrics::counter!(
        AGENT_RUNS_TOTAL,
        "outcome" => outcome,
        "agent_type" => agent_label(agent_type),
    )
    .increment(1);
}

/// Record one run's agent spend (USD) into the cost histogram.
pub(crate) fn record_cost(agent_type: Option<&str>, usd: f64) {
    if usd <= 0.0 {
        return;
    }
    metrics::histogram!(
        AGENT_COST_USD,
        "agent_type" => agent_label(agent_type),
    )
    .record(usd);
}

/// Record turn wall-clock duration (seconds). `context` must already be
/// bucketed ([`context_bucket`]).
pub(crate) fn record_turn_duration(
    agent_type: Option<&str>,
    outcome: &'static str,
    context: &'static str,
    seconds: f64,
) {
    metrics::histogram!(
        AGENT_TURN_DURATION_SECONDS,
        "agent_type" => agent_label(agent_type),
        "outcome" => outcome,
        "context_bucket" => context,
    )
    .record(seconds);
}

/// Record one run's token consumption for a single `kind`. A missing or
/// non-positive count is not a zero-valued observation — it is the runtime not
/// reporting that class at all, so it is skipped rather than counted.
pub(crate) fn record_tokens(agent_type: Option<&str>, kind: &'static str, tokens: Option<i64>) {
    let Some(tokens) = tokens
        .and_then(|t| u64::try_from(t).ok())
        .filter(|&t| t > 0)
    else {
        return;
    };
    metrics::counter!(
        AGENT_TOKENS_TOTAL,
        "agent_type" => agent_label(agent_type),
        "kind" => kind,
    )
    .increment(tokens);
}

/// Record turn API time (seconds).
pub(crate) fn record_turn_api_duration(agent_type: Option<&str>, seconds: f64) {
    metrics::histogram!(
        AGENT_TURN_API_DURATION_SECONDS,
        "agent_type" => agent_label(agent_type),
    )
    .record(seconds);
}

/// Record the number of internal turns in an agent run.
pub(crate) fn record_turns(agent_type: Option<&str>, num_turns: u32) {
    metrics::histogram!(
        AGENT_TURNS,
        "agent_type" => agent_label(agent_type),
    )
    .record(f64::from(num_turns));
}

/// Record one tool-permission denial. `tool` must already be bucketed.
pub(crate) fn record_permission_denial(tool: &'static str) {
    metrics::counter!(
        AGENT_PERMISSION_DENIALS_TOTAL,
        "tool" => tool,
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_tool_keeps_known_builtins() {
        assert_eq!(bucket_tool("Bash"), "Bash");
        assert_eq!(bucket_tool("Read"), "Read");
        assert_eq!(bucket_tool("Skill"), "Skill");
        assert_eq!(bucket_tool("ExitPlanMode"), "ExitPlanMode");
    }

    #[test]
    fn bucket_tool_collapses_mcp() {
        assert_eq!(bucket_tool("mcp__github__create_issue"), "mcp");
        assert_eq!(bucket_tool("mcp__atlassian__search"), "mcp");
    }

    #[test]
    fn bucket_tool_collapses_unknown_to_other() {
        assert_eq!(bucket_tool("SomeRandomTool"), "other");
        assert_eq!(bucket_tool(""), "other");
        // A skill *name* must not leak through as a tool label — the tool is
        // always "Skill"; the skill name lives in the input, not the name.
        assert_eq!(bucket_tool("deploy"), "other");
    }

    #[test]
    fn agent_label_defaults_to_unknown() {
        assert_eq!(agent_label(None), "unknown");
        assert_eq!(agent_label(Some("")), "unknown");
        assert_eq!(agent_label(Some("claude")), "claude");
    }

    /// Usage carrying only the input side, which is all `context_bucket` reads.
    fn input_usage(input: i64, cache_read: i64, cache_creation: i64) -> UsageInfo {
        UsageInfo {
            input_tokens: Some(input),
            output_tokens: Some(9_999_999),
            cache_creation_input_tokens: Some(cache_creation),
            cache_read_input_tokens: Some(cache_read),
        }
    }

    #[test]
    fn context_bucket_is_unknown_without_usable_usage() {
        assert_eq!(context_bucket(None, Some(4)), "unknown");
        assert_eq!(
            context_bucket(Some(&input_usage(0, 0, 0)), Some(4)),
            "unknown"
        );
    }

    #[test]
    fn context_bucket_sums_the_whole_input_side() {
        // 10k prompt + 120k cache read + 30k cache creation = 160k on one turn.
        assert_eq!(
            context_bucket(Some(&input_usage(10_000, 120_000, 30_000)), Some(1)),
            "100-200k"
        );
    }

    #[test]
    fn context_bucket_divides_the_run_total_by_its_turns() {
        // 2M input tokens is the top bucket over 1 turn and a mid one over 20 —
        // this division is the difference between the label meaning "prompt
        // size" and meaning "how long the run was".
        let usage = input_usage(2_000_000, 0, 0);
        assert_eq!(context_bucket(Some(&usage), Some(1)), "400k+");
        assert_eq!(context_bucket(Some(&usage), Some(20)), "100-200k");
    }

    #[test]
    fn context_bucket_treats_a_missing_or_zero_turn_count_as_one_call() {
        let usage = input_usage(500_000, 0, 0);
        assert_eq!(context_bucket(Some(&usage), None), "400k+");
        assert_eq!(context_bucket(Some(&usage), Some(0)), "400k+");
    }

    #[test]
    fn context_bucket_labels_sort_lexically_in_numeric_order() {
        let ascending = [
            context_bucket(Some(&input_usage(10_000, 0, 0)), Some(1)),
            context_bucket(Some(&input_usage(70_000, 0, 0)), Some(1)),
            context_bucket(Some(&input_usage(150_000, 0, 0)), Some(1)),
            context_bucket(Some(&input_usage(300_000, 0, 0)), Some(1)),
            context_bucket(Some(&input_usage(900_000, 0, 0)), Some(1)),
        ];
        assert_eq!(
            ascending,
            ["000-050k", "050-100k", "100-200k", "200-400k", "400k+"]
        );
        let mut sorted = ascending;
        sorted.sort_unstable();
        assert_eq!(sorted, ascending, "label order must survive a lexical sort");
    }
}

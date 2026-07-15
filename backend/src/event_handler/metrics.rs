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

/// Wall-clock turn duration in seconds by `agent_type` and `outcome`
/// (histogram). Sourced from the SDK result's `duration_ms`.
pub(crate) const AGENT_TURN_DURATION_SECONDS: &str = "moatless_agent_turn_duration_seconds";

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
        "Agent turn wall-clock duration in seconds by agent_type and outcome"
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

/// Record turn wall-clock duration (seconds).
pub(crate) fn record_turn_duration(agent_type: Option<&str>, outcome: &'static str, seconds: f64) {
    metrics::histogram!(
        AGENT_TURN_DURATION_SECONDS,
        "agent_type" => agent_label(agent_type),
        "outcome" => outcome,
    )
    .record(seconds);
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
}

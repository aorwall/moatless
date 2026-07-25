//! Linear adapter — Agents-first integration (mention / assignment driven
//! `AgentSessionEvent` webhooks). Maps Linear workspaces to integration
//! connections and sessions to task bindings.

pub mod active_turns;
pub mod adapter;
pub mod agent_api;
pub mod agent_runtime;
pub mod api_routes;
pub mod config;
pub mod handlers;
pub mod signature;
pub mod status;
pub mod webhook_payloads;

/// `task_bindings.subject_kind` for bindings that point to a Linear issue
/// (CLI-created or operator-initiated). Distinct from the existing
/// `linear_agent_session` kind, which routes replies through
/// `agentActivityCreate`; `linear_issue` routes through `commentCreate`.
pub const SUBJECT_KIND_ISSUE: &str = "linear_issue";

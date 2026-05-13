//! Linear adapter — Agents-first integration (mention / assignment driven
//! `AgentSessionEvent` webhooks). Maps Linear workspaces to integration
//! connections and sessions to task bindings.

pub mod adapter;
pub mod agent_api;
pub mod agent_runtime;
pub mod config;
pub mod handlers;
pub mod signature;
pub mod webhook_payloads;

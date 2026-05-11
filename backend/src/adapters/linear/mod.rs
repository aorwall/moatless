//! Linear adapter — maps workspaces to adapter connections, teams to
//! task triggers (multiple per team allowed), issues to task bindings.

pub mod adapter;
pub mod api;
pub mod config;
pub mod handlers;
pub mod runtime;
pub mod signature;

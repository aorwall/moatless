//! Linear channel provider — maps workspaces to provider connections, teams to
//! ingress rules (multiple per team allowed), issues to task subscriptions.

pub mod api;
pub mod config;
pub mod provider;
pub mod runtime;
pub mod signature;

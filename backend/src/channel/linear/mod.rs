//! Linear channel provider — maps workspaces to provider connections, teams to
//! task triggers (multiple per team allowed), issues to task bindings.

pub mod api;
pub mod config;
pub mod handlers;
pub mod provider;
pub mod runtime;
pub mod signature;

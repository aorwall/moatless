//! GitHub PR adapter — maps repos to trigger sources and PRs to task bindings.

pub mod adapter;
pub mod api;
pub mod api_routes;
pub mod config;
pub mod external_id;
pub mod handlers;
pub mod runtime;
pub mod signals;
pub mod signature;
pub mod status;

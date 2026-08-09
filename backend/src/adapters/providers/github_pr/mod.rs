//! GitHub PR adapter — maps repos to trigger sources and PRs to task bindings.

pub(crate) mod adapter;
pub(crate) mod api;
pub(crate) mod api_routes;
mod auth;
mod config;
pub(crate) mod delivery;
mod external_id;
pub(crate) mod handlers;
mod ingress;
mod merge_conflict;
mod outbound;
mod post_merge;
mod reactions;
pub(crate) mod router;
mod routing;
pub(crate) mod shell_reconcile;
mod signals;
mod signature;
mod status;
mod subject;
#[cfg(test)]
mod test_support;
mod text;
mod webhook;

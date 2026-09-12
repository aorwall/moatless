#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! T3 wire compatibility over Moatless-owned operations and scoped read projections.

pub mod activity;
pub(crate) mod activity_dao;
pub mod agent;
mod assets;
mod attachments;
pub mod auth;
mod authentication;
mod checkout;
mod commands;
mod compose;
mod config;
pub(crate) mod dao;
mod feed;
pub mod files;
mod git_actions;
mod handlers;
pub mod listing;
mod nominal_path;
mod oauth;
mod observability;
pub mod ports;
mod preview;
pub mod preview_tabs;
mod projection;
mod providers;
mod reads;
mod review;
mod routes;
mod rpc;
mod sandbox;
pub mod scripts;
mod servers;
mod skills;
mod snapshots;
mod startup_scripts;
mod state;
pub(crate) mod subscription;
mod subscriptions;
mod subtasks;
pub(crate) mod task_dao;
mod terminal;
mod usage;
pub mod usage_limit_sources;
pub mod workspace;
pub(crate) mod workspace_dao;

pub(crate) use authentication::viewer_scope;
pub use checkout::Sources;
pub use compose::T3Graph;
pub use compose::UiRpc;
pub use compose::UiRpcDependencies;
pub use compose::router_from_env;
pub use compose::t3_graph;
pub use config::AgentModelDefaults;
pub use config::UiRpcConfigError;
pub use feed::{ShellFeed, ThreadFeed};
pub use moatless_identity::auth::pairing::DevicePairing;
pub use moatless_ports::sandbox::{
    ClusterDiagnostics, SandboxIntentReader, SandboxObserver, SandboxPortPublisher, SandboxStarter,
    SandboxStopper, WorkspaceSnapshotSource,
};
pub use moatless_types::ui_rpc::{UiRpcAuthIdentity, UiRpcAuthMethod, UserRole};
pub use reads::T3Reads;
pub(crate) use rpc::frames::RequestId;
pub(crate) use rpc::frames::failure_exit;
pub(crate) use rpc::frames::sandbox_stream_refusal;
pub(crate) use rpc::frames::success_exit;
pub(crate) use rpc::session::MAX_ACTIVE_STREAMS;
pub(crate) use rpc::session::MAX_DETACHED_REQUESTS;
pub use sandbox_proto::client::SandboxConnector;
pub use snapshots::ShellSnapshot;
pub use snapshots::ThreadSnapshot;
pub use snapshots::archived_shell_snapshot;
pub use snapshots::shell_snapshot;
pub(crate) use snapshots::source_task_names;
pub use snapshots::thread_shell;
pub use snapshots::thread_snapshot;
pub use startup_scripts::{StartupScriptFailure, StartupScripts};
pub use state::Deployment;
pub use state::UiRpcDbPool;
pub(crate) use state::UiRpcState;
pub use subscriptions::ShellSubscription;
pub use subscriptions::shell_subscription;
pub use subscriptions::thread_subscription;

#[cfg(test)]
mod tests;

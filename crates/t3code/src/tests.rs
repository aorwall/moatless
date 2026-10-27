//! Unit tests for the RPC server, one module per subject.

use crate::config::AgentModelDefaults;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::config::UiRpcConfig;
use crate::config::environment_id;
use crate::dao::ConversationDao;
use crate::dao::MockConversationDao;
use crate::handlers::origin_allowed;
use crate::handlers::origin_checked;
use crate::oauth::Scope;
use crate::observability;
use crate::ports;
use crate::preview::PreviewState;
use crate::preview_relay;
use crate::preview_tabs;
use crate::reads::T3Reads;
use crate::routes::router;
use crate::rpc::config::descriptor;
use crate::rpc::config::server_config;
use crate::rpc::dispatch::methods;
use crate::rpc::envelope::RequestId;
use crate::rpc::errors::THREAD_UNAVAILABLE;
use crate::rpc::errors::sandbox_failure_exit;
use crate::rpc::errors::sandbox_stream_refusal;
use crate::rpc::errors::scripts_failure_exit;
use crate::rpc::metrics as rpc_metrics;
use crate::rpc::metrics::record_request;
use crate::rpc::metrics::remote_span_context;
use crate::rpc::metrics::request_root_span;
use crate::rpc::session::MAX_ACTIVE_STREAMS;
use crate::rpc::session::ProtocolOutput;
use crate::rpc::session::ProtocolSession;
use crate::rpc::socket::MAX_FRAME_BYTES;
use crate::rpc::socket::OUTBOUND_FRAMES;
use crate::sandbox::SandboxRpcError;
use crate::sandbox::SandboxState;
use crate::scripts::ScriptsError;
use crate::scripts::ScriptsState;
use crate::state::UiRpcState;
use crate::subscriptions::OrchestrationV2SubscribeShellInput;
use crate::subscriptions::thread_initial_values;
use crate::subtasks::SubtasksState;
use crate::terminal;
use crate::workspace::WorkspaceAccessError;
use crate::workspace_listing::MockWorkspaceListing;
use async_trait::async_trait;
use axum::extract::Extension;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use chrono::{DateTime, Utc};
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_core::change_feed::listing::EnvironmentIdentity;
use moatless_core::change_feed::listing::ListingHead;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_identity::auth::pairing::{
    PairedDevice, PairingApiKeyIssuer, PairingCode, PairingError, device_pairing_over, hash_key,
};
use moatless_ports::sandbox::ClusterDiagnostics;
use moatless_ports::sandbox::SandboxIntentReader;
use moatless_ports::sandbox::SandboxObserver;
use moatless_ports::sandbox::SandboxStarter;
use moatless_ports::sandbox::SandboxStopper;
use moatless_ports::sandbox::{
    ContractSandboxStatus, LiveStatus, SandboxDesiredState, SandboxDiagnosticsContainer,
    SandboxDiagnosticsEntry, SandboxError, SandboxPortPublisher, SandboxStatus,
};
use moatless_ports::task::{MockTaskReader, TaskDescriptor};
use moatless_types::access_control::ScopeContext;
use moatless_types::auth::AllowedOrigin;
use moatless_types::task::TaskStatus;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use moatless_types::ui_rpc::UiRpcAuthMethod;
use moatless_types::ui_rpc::UserRole;
use sandbox_proto::client::SandboxConnector;
use serde_json::Value;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use uuid::Uuid;

/// A readable open Task. Inert but for the id: the gates read whether a scoped
/// read returned a Task, never a column on it.
pub(crate) fn open_task(task_id: &str) -> TaskDescriptor {
    TaskDescriptor {
        task_id: task_id.to_string(),
        status: TaskStatus::Open,
        name: None,
        owner_user_id: None,
        workspace_id: None,
        created_by: None,
        ttl_minutes: 15,
    }
}

pub(crate) mod fixtures;
pub(crate) mod logs;

mod artifacts;
mod assets;
mod attachment_uploads;
mod attachments;
mod config;
mod descriptor;
mod diagnostics;
mod files;
mod http_compression;
mod listing;
pub(crate) mod methods;
mod origin;
mod pairing;
mod refusals;
mod request_ids;
mod sandbox_status;
mod scripts;
mod subscriptions;
mod trace_context;
mod vcs;
mod workspace_assets;

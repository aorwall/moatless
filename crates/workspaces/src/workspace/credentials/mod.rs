//! Provider credentials pinned to a Workspace.
//!
//! Outranks the run-as user's `provider_tokens` row and any global credential,
//! letting a Workspace run as a dedicated machine account. One value per
//! `(workspace, provider)`, seeded first into sandbox credential resolution
//! (see `sandbox-control`'s `secrets::resolve_task_credentials`).
//!
//! A git-host pin ([`WorkspaceCredentialProvider::GIT_HOSTS`]) also governs
//! backend calls via `GitHostTokens`, so sandbox and backend present the same
//! account. Values are envelope-encrypted; only
//! [`service::WorkspaceCredentialService::resolve_env`] decrypts, for the
//! sandbox.

pub(crate) mod dao;
pub(crate) mod handlers;
pub mod models;
pub mod service;

pub use models::{WorkspaceCredential, WorkspaceCredentialMetadata, WorkspaceCredentialWrite};
pub use service::WorkspaceCredentialService;

pub use moatless_workspace_types::schemas::workspace::{
    SetWorkspaceCredentialRequest, WorkspaceCredentialResponse,
};
pub use moatless_workspace_types::workspace::WorkspaceCredentialProvider;

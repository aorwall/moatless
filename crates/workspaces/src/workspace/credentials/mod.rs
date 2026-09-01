//! Provider credentials pinned to a Workspace.
//!
//! A Workspace credential is the credential every Task on that Workspace
//! authenticates with, whoever the Task runs as. It outranks the run-as user's
//! `provider_tokens` row and any global credential, which is what lets a
//! Workspace run against a dedicated machine account rather than borrowing
//! whichever human happened to start the Task.
//!
//! One value per `(workspace, provider)`, delivered to the sandbox as the one
//! environment variable that provider maps to
//! ([`WorkspaceCredentialProvider::env_key`]). Sandbox credential resolution
//! seeds its map from these before consulting anything else — see
//! `sandbox-control`'s `secrets::resolve_task_credentials`.
//!
//! A pin for a git host ([`WorkspaceCredentialProvider::GIT_HOSTS`]) governs
//! backend-side calls too: an actor carrying the Workspace resolves it through
//! `moatless_identity::git_host_tokens::GitHostTokens`, so the sandbox cloning
//! and the backend opening the pull request present the same account.
//!
//! Values are envelope-encrypted with the cipher the `secrets` table uses, and
//! no read path returns one: only
//! [`service::WorkspaceCredentialService::resolve_env`] decrypts, for the
//! sandbox.

pub mod dao;
pub mod handlers;
pub mod models;
pub mod service;

pub use models::{WorkspaceCredential, WorkspaceCredentialMetadata, WorkspaceCredentialWrite};
pub use service::WorkspaceCredentialService;

pub use moatless_types::workspace::WorkspaceCredentialProvider;
pub use moatless_api_schemas::workspace::{
    SetWorkspaceCredentialRequest, WorkspaceCredentialResponse,
};

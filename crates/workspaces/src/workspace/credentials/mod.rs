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
//! Values are envelope-encrypted with the cipher the `secrets` table uses, and
//! no read path returns one: [`service::WorkspaceCredentialService::resolve_env`]
//! decrypts for the sandbox, everything else returns metadata.

pub mod dao;
pub mod handlers;
pub mod models;
pub mod service;

pub use models::{WorkspaceCredential, WorkspaceCredentialWrite};
pub use service::WorkspaceCredentialService;

pub use moatless_api_schemas::workspace::{
    SetWorkspaceCredentialRequest, WorkspaceCredentialProvider, WorkspaceCredentialResponse,
};

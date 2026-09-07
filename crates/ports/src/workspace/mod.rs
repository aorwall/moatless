//! Contracts published by the workspace domain.

pub use moatless_types::plugin::PluginDelivery;
pub use moatless_types::repository::Repository;

mod resolution;
#[cfg(feature = "test-support")]
pub use resolution::MockWorkspaceResolver;
pub use resolution::{RepoPlacement, ResolvedWorkspace, WorkspaceResolver};

mod credentials;
#[cfg(feature = "test-support")]
pub use credentials::MockWorkspaceCredentialReader;
pub use credentials::WorkspaceCredentialReader;

mod repository;
#[cfg(feature = "test-support")]
pub use repository::MockRepositoryReader;
pub use repository::RepositoryReader;

mod provisioning;
#[cfg(feature = "test-support")]
pub use provisioning::MockWorkspaceProvisioner;
pub use provisioning::WorkspaceProvisioner;

//! Contracts published by the identity domain.

mod user;
#[cfg(feature = "test-support")]
pub use user::{MockUserDisplay, MockUserResolver};
pub use user::{RunAsUser, UserDisplay, UserResolver};

mod provider_link;
#[cfg(feature = "test-support")]
pub use provider_link::MockProviderIdentityLink;
pub use provider_link::{LinkedUser, ProviderIdentityLink};

mod commit;
#[cfg(feature = "test-support")]
pub use commit::MockCommitIdentity;
pub use commit::{CommitIdentity, GitIdentity};

mod credentials;
pub use credentials::{AppCredentials, AppToken, CodexScopeReader, HarnessCredentials};
#[cfg(feature = "test-support")]
pub use credentials::{MockAppCredentials, MockCodexScopeReader, MockHarnessCredentials};

mod secret;
#[cfg(feature = "test-support")]
pub use secret::{MockSecretCatalog, MockSecretResolver};
pub use secret::{SecretCatalog, SecretResolver};

mod git_host;
#[cfg(feature = "test-support")]
pub use git_host::MockGitHostTokenIssuer;
pub use git_host::{GitHostTokenError, GitHostTokenIssuer, PlainGitHost};

use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use std::sync::Arc;
/// Shared configuration for the lifetime of a Codex harness.
#[derive(Clone)]
pub struct CodexConfig {
    pub config: Arc<Config>,
    /// Shared per-credential failure flags.
    ///
    /// A Turn that fails with the named unauthorized-request code flips the
    /// Codex flag, which the coordinator polls to drive a credential pull +
    /// retry, keeping credential recovery behind the harness.
    pub credential_failures: CredentialFailureDetector,
    /// Where each Session's file is recorded; `None` records nothing.
    pub trajectory: Option<Arc<crate::trajectory_tail::TrajectoryRecording>>,
}

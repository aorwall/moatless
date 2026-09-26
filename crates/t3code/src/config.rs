use crate::listing::UiRpcEnvironmentIdentity;
use moatless_types::agent::Harness;
use moatless_types::auth::AllowedOrigin;
use std::env;

/// The T3 Code client release this module's surface matches; a client refuses a
/// server reporting a different version.
///
/// Move it in the change that brings the surface up to a newer client.
pub(crate) const T3_CLIENT_CONTRACT_VERSION: &str = "0.0.31";

/// The readable half of the environment id, and the name the client displays.
/// The digest [`environment_id`] appends is what tells environments apart.
pub(crate) const T3_ENVIRONMENT_NAME: &str = "moatless";
pub(crate) const T3_ENVIRONMENT_LABEL: &str = "Moatless";

#[derive(Debug, Clone)]
pub(crate) struct UiRpcConfig {
    pub(crate) environment_id: String,
    pub(crate) allowed_origins: Vec<AllowedOrigin>,
    /// Whether this deployment exports spans over OTLP — reported to the client
    /// as `observability.otlpTracesEnabled` (see [`server_config`]).
    pub(crate) otlp_traces_enabled: bool,
    /// Whether attachment uploads can be offered — whether a URL can be signed
    /// for one; see [`attachments`]. Reported as
    /// `capabilities.attachmentUploads`.
    ///
    /// Must be reported honestly: the composer stops sending images inline once
    /// it reads this true, so a deployment that then couldn't take the upload
    /// would lose the attachment.
    pub(crate) attachment_uploads: bool,
    /// Whether a `CLIProxyAPI` hub is configured, and therefore whether the
    /// config stream ever emits `usageLimitSourcesUpdated` — see
    /// [`crate::usage_limit_sources`].
    ///
    /// Reported as `capabilities.usageLimitSources`, which the client reads
    /// before it will accept the event at all.
    pub(crate) usage_limit_sources: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum UiRpcConfigError {
    #[error("Invalid T3 UI RPC configuration: {0}")]
    Invalid(String),
    /// The environment's identity is read from the database; a database that
    /// cannot be read has no identity, so this module cannot be mounted.
    ///
    /// Fails startup rather than falling back to a configured constant, since
    /// silently reverting would present a replaced database as the same
    /// environment — the exact failure the identity exists to make visible.
    #[error("The T3 UI RPC environment identity could not be read: {0}")]
    Unidentifiable(#[from] sqlx::Error),
}

/// The models this deployment forwards to Sandboxes when a conversation names
/// none, by kind of agent.
///
/// Step three of the chain a conversation's reported model is resolved through;
/// both fields are optional configuration, so both may be absent.
#[derive(Debug, Clone, Default)]
pub struct AgentModelDefaults {
    pub claude_code: Option<String>,
    pub codex: Option<String>,
}

impl AgentModelDefaults {
    /// The deployment default for one harness, if it configures one.
    pub(crate) fn for_harness(&self, harness: Harness) -> Option<&str> {
        match harness {
            Harness::ClaudeCode => self.claude_code.as_deref(),
            Harness::Codex => self.codex.as_deref(),
        }
    }
}

/// The identity this deployment presents itself by: a configured name and a
/// digest of the database behind it — two deployments sharing a database are
/// one environment.
///
/// Cached per environment and read once per page load, so a changed id is only
/// safe mid-session because [`task_listing::resume`] and
/// [`ShellFeed::poll`](feed::ShellFeed::poll) guard the mismatch.
pub(crate) fn environment_id(identity: &UiRpcEnvironmentIdentity) -> String {
    format!("{T3_ENVIRONMENT_NAME}-{}", identity.digest())
}

impl UiRpcConfig {
    pub(crate) fn validate(&self) -> Result<(), UiRpcConfigError> {
        if self.allowed_origins.is_empty() {
            return Err(UiRpcConfigError::Invalid(
                "T3_UI_RPC_ALLOWED_ORIGINS must contain at least one origin".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn env_enabled(key: &str) -> Result<bool, UiRpcConfigError> {
    match env::var(key) {
        Err(env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(UiRpcConfigError::Invalid(format!(
            "{key} could not be read: {error}"
        ))),
        Ok(value) => parse_enabled(key, &value),
    }
}

pub(crate) fn parse_enabled(key: &str, value: &str) -> Result<bool, UiRpcConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(UiRpcConfigError::Invalid(format!(
            "{key} must be true or false"
        ))),
    }
}

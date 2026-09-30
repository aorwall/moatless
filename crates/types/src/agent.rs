//! Agent harness vocabulary: the credential an agent session runs under, and
//! what its events say.
//!
//! The Codex `auth.json` an agent session runs with is an agent harness
//! credential, not a provider token a user signs in with, so its settings and
//! its credential-row discriminator live here rather than in `auth`.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use serde::{Deserialize, Serialize};

/// Codex (OpenAI) device-code OAuth configuration.
///
/// Defaults reuse the Codex CLI's public `client_id`; override via env vars to
/// register a Moatless-owned OAuth client.
#[derive(Debug, Clone)]
pub struct CodexOAuthConfig {
    /// OAuth issuer base URL (default: `https://auth.openai.com`).
    pub issuer: String,
    /// OAuth `client_id` (default: Codex CLI's `app_EMoamEEZ73f0CkXaXp7hrann`).
    pub client_id: String,
    /// The refresh-token endpoint a stored Codex login is rotated against.
    pub refresh_url: String,
}

// ---------------------------------------------------------------------------
// CodexScope — which Codex credential row an operation addresses
// ---------------------------------------------------------------------------

/// Discriminator + identifier for a Codex `provider_tokens` row's scope.
///
/// Drives the SQL `WHERE` fragment, advisory-lock key, and per-row mutex key
/// that address that row; resolver precedence is `User > Global`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexScope {
    /// Per-user row: `(user_id = id, scope = 'user')`.
    User(String),
    /// Singleton global row: `(user_id IS NULL, team_id IS NULL, scope =
    /// 'global')`.
    Global,
}

impl CodexScope {
    /// Convenience constructors.
    pub fn user(id: impl Into<String>) -> Self {
        Self::User(id.into())
    }

    /// Short human-readable label for log fields (`scope=user`,
    /// `scope=global`).
    pub fn display(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Global => "global".to_string(),
        }
    }

    /// Scope kind as the literal stored in `provider_tokens.scope`.
    pub fn scope_kind(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for CodexScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

crate::string_enum!(
    /// Agent sub-types.
    AgentSubType [Hash] {
        Started => "started",
        Stopped => "stopped",
        Completed => "completed",
        Error => "error",
        PendingResponse => "pending_response",
        SessionInit => "session_init",
        Retrying => "retrying",
        Scheduled => "scheduled",
        /// Sandbox restarted mid-run and is replaying the previously interrupted
        /// turn via `--resume <session_id>`.
        RecoveryStarted => "recovery_started",
        /// Recovery attempts exhausted for an interrupted turn — the run has been
        /// abandoned and an Aborted Result row was inserted in its place.
        RecoveryExhausted => "recovery_exhausted",
        /// A session resume failed because the transcript is unavailable (e.g.
        /// stale session id after a pod restart); the agent fell back to a fresh
        /// session. Previous conversation context is lost. Warning, not an error.
        SessionContextLost => "session_context_lost",
        /// Liveness marker published (rate-limited) during a long-running tool call
        /// so the reaper's activity floor keeps advancing.
        ///
        /// Carries no conversation content; SSE-only, never persisted as a message.
        Activity => "activity",
        /// What the harness said about its own context window: how much of it the
        /// session occupies, and how large it is where the harness states one.
        ContextWindow => "context_window",
        /// Count of running `moat cmd` commands, published whenever it changes.
        ///
        /// Absolute, not a delta — the registry is in-memory, so a pod replacement
        /// would strand a decrement. Says nothing about the agent; see
        /// [`crate::event::agent_working`].
        CommandsChanged => "commands_changed",
        /// A trajectory recorder stopped, so the rest of that harness session
        /// is missing from the Trajectory. The Turn goes on.
        RecordingStopped => "recording_stopped",
    }
);

impl AgentSubType {
    /// Whether this sub-type represents an error condition.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error | Self::RecoveryExhausted)
    }
}

/// Agent event payload — typed shape for `EventType::Agent` events.
///
/// Pass-through from the sandbox; primarily for OpenAPI schema generation,
/// since the backend forwards raw `serde_json::Value` payloads at runtime.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentEventPayload {
    /// The message role type: "assistant", "user", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    /// For "completed" subtype: the SDK result.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub result: Option<serde_json::Value>,
    /// For "`pending_response`" subtype: the tool being invoked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub tool_input: Option<serde_json::Value>,
    /// Context window usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    /// Cumulative cost in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    /// The agent session ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

/// The Anthropic model id each Claude catalog alias names.
///
/// What the Sandbox passes to the Claude CLI, since a proxy routing on the id
/// answers an alias with `unknown provider for model opus`. These are ids such
/// an API accepts, not the slugs T3 publishes — `haiku` is the one that
/// differs, since `claude-haiku-4-5` is refused.
const CLAUDE_MODEL_IDS: &[(&str, &str)] = &[
    ("opus", "claude-opus-5-5"),
    ("sonnet", "claude-sonnet-5"),
    ("haiku", "claude-haiku-4-5-20251001"),
    ("fable", "claude-fable-5-1"),
];

/// The model id a Claude catalog alias names, or `None` for anything that is
/// not an alias — an id already, or a choice naming no model.
pub fn claude_model_id(alias: &str) -> Option<&'static str> {
    CLAUDE_MODEL_IDS
        .iter()
        .find(|(name, _)| *name == alias)
        .map(|(_, id)| *id)
}

/// The Claude catalog alias a model id belongs to, or `None` for an id no
/// catalog entry names.
pub fn claude_model_alias(model_id: &str) -> Option<&'static str> {
    CLAUDE_MODEL_IDS
        .iter()
        .find(|(_, id)| *id == model_id)
        .map(|(alias, _)| *alias)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_kind_matches_db_literal() {
        assert_eq!(CodexScope::user("a").scope_kind(), "user");
        assert_eq!(CodexScope::Global.scope_kind(), "global");
    }

    #[test]
    fn display_is_compact() {
        assert_eq!(CodexScope::user("a").display(), "user:a");
        assert_eq!(CodexScope::Global.display(), "global");
    }

    #[test]
    fn agent_sub_type_serializes() {
        assert_eq!(
            serde_json::to_string(&AgentSubType::PendingResponse).unwrap(),
            "\"pending_response\""
        );
    }

    #[test]
    fn agent_sub_type_session_context_lost_round_trips() {
        assert_eq!(
            AgentSubType::SessionContextLost.as_str(),
            "session_context_lost"
        );
        assert_eq!(
            serde_json::to_string(&AgentSubType::SessionContextLost).unwrap(),
            "\"session_context_lost\""
        );
        assert_eq!(
            AgentSubType::try_from("session_context_lost"),
            Ok(AgentSubType::SessionContextLost)
        );
        // A warning, not an error: the agent continues with a fresh session.
        assert!(!AgentSubType::SessionContextLost.is_error());
    }

    #[test]
    fn agent_sub_type_activity_round_trips() {
        assert_eq!(AgentSubType::Activity.as_str(), "activity");
        assert_eq!(
            serde_json::to_string(&AgentSubType::Activity).unwrap(),
            "\"activity\""
        );
        assert_eq!(
            AgentSubType::try_from("activity"),
            Ok(AgentSubType::Activity)
        );
        assert!(!AgentSubType::Activity.is_error());
    }

    #[test]
    fn agent_sub_type_try_from() {
        assert_eq!(
            AgentSubType::try_from("completed"),
            Ok(AgentSubType::Completed)
        );
        assert_eq!(
            AgentSubType::try_from("session_init"),
            Ok(AgentSubType::SessionInit)
        );
        assert!(AgentSubType::try_from("bogus").is_err());
    }
}

/// The executable family that runs an Agent. The only harness type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
}

impl Harness {
    /// The wire and column value: `claude-code` or `codex`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }

    /// Parse a wire or column value; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

impl std::fmt::Display for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A catalog entry: a harness plus transport, default model and credentials (e.g. "claude-code-tui").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct AgentProfile(pub String);

impl AgentProfile {
    pub const CLAUDE_CODE: &str = "claude-code";
    pub const CLAUDE_CODE_TUI: &str = "claude-code-tui";
    pub const CLAUDE_CODE_CLI: &str = "claude-code-cli";
    pub const CLAUDE_CODE_FABLE: &str = "claude-code-fable";
    pub const CODEX: &str = "codex";
    pub const SUPPORTED: &'static [&'static str] = &[
        Self::CLAUDE_CODE,
        Self::CLAUDE_CODE_TUI,
        Self::CLAUDE_CODE_CLI,
        Self::CLAUDE_CODE_FABLE,
        Self::CODEX,
    ];

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for AgentProfile {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AgentProfile {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Agent mode identifier (open-ended, e.g. "plan", "code").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct AgentMode(pub String);

impl AgentMode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for AgentMode {
    fn from(value: String) -> Self {
        Self(value)
    }
}

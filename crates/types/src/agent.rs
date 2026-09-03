//! Agent harness vocabulary: the credential an agent session runs under,
//! and what its events say.
//!
//! The Codex `auth.json` an agent session runs with is an agent harness
//! credential, not a provider token a user signs in with, so its settings
//! and its credential-row discriminator live here rather than in `auth`.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use serde::{Deserialize, Serialize};

/// Codex (OpenAI) device-code OAuth configuration.
///
/// Defaults reuse the Codex CLI's public `client_id` so the flow works out of
/// the box. Override via env vars to register a Moatless-owned OAuth client.
#[derive(Debug, Clone)]
pub struct CodexOAuthConfig {
    /// OAuth issuer base URL (default: `https://auth.openai.com`).
    pub issuer: String,
    /// OAuth `client_id` (default: Codex CLI's `app_EMoamEEZ73f0CkXaXp7hrann`).
    pub client_id: String,
}

// ---------------------------------------------------------------------------
// CodexScope — which Codex credential row an operation addresses
// ---------------------------------------------------------------------------

/// Discriminator + identifier for a Codex `provider_tokens` row's scope.
///
/// It is the discriminant that drives every scope-aware operation
/// in the Codex refresh pipeline: the SQL `WHERE` fragment that targets the
/// right row, the Postgres advisory-lock key, and the in-process per-row mutex
/// key. The variants intentionally carry the scope identifier so a single
/// value uniquely addresses one row without callers having to thread
/// `(scope_kind, scope_id)` tuples through every layer.
///
/// Two scopes:
/// - [`CodexScope::User`] — per-user row keyed by `user_id`.
/// - [`CodexScope::Global`] — single global row: `user_id IS NULL`,
///   `team_id IS NULL`, `scope = 'global'`.
///
/// Resolver precedence: `User > Global` (most-specific wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexScope {
    /// Per-user row: `(user_id = id, scope = 'user')`.
    User(String),
    /// Singleton global row: `(user_id IS NULL, team_id IS NULL, scope = 'global')`.
    Global,
}

impl CodexScope {
    /// Convenience constructors.
    pub fn user(id: impl Into<String>) -> Self {
        Self::User(id.into())
    }

    /// The string the Postgres advisory lock is hashed from. Includes the
    /// scope discriminant so a user named `"foo"` doesn't collide with the
    /// global lock.
    pub fn advisory_id_input(&self) -> String {
        match self {
            Self::User(id) => format!("codex-refresh:user:{id}"),
            Self::Global => "codex-refresh:global".to_string(),
        }
    }

    /// Alias for [`advisory_id_input`]; reads naturally at call sites that
    /// also use it as the in-process mutex discriminator.
    pub fn lock_key(&self) -> String {
        self.advisory_id_input()
    }

    /// Short human-readable label for log fields (`scope=user`, `scope=global`).
    pub fn display(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Global => "global".to_string(),
        }
    }

    /// Bind value for the `WHERE` fragment's single `$N` placeholder.
    /// User scope carries the `user_id`; Global uses an empty string sentinel
    /// (the clause itself uses IS NULL checks, not a bound value).
    pub fn bind_value(&self) -> &str {
        match self {
            Self::User(id) => id.as_str(),
            Self::Global => "",
        }
    }

    /// Build the `WHERE` clause fragment that addresses this scope's row in
    /// `provider_tokens`, using `placeholder` for the dynamic identifier.
    /// Callers paste the result into the larger SQL string and bind
    /// [`bind_value`] at the same positional parameter index.
    ///
    /// Note: Global scope uses `IS NULL` checks so the bind value is unused
    /// for that variant; callers should still bind it (empty string) to keep
    /// positional parameter counts consistent.
    pub fn where_clause(&self, placeholder: &str) -> String {
        match self {
            Self::User(_) => {
                format!("user_id = {placeholder} AND provider = 'codex' AND scope = 'user'")
            }
            Self::Global => {
                "user_id IS NULL AND team_id IS NULL AND provider = 'codex' AND scope = 'global'"
                    .to_string()
            }
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

/// Agent sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AgentSubType {
    Started,
    Stopped,
    Message,
    Completed,
    Error,
    PendingResponse,
    SessionInit,
    Retrying,
    Scheduled,
    /// Sandbox restarted mid-run and is replaying the previously interrupted
    /// turn via `--resume <session_id>`.
    RecoveryStarted,
    /// Recovery attempts exhausted for an interrupted turn — the run has been
    /// abandoned and an Aborted Result row was inserted in its place.
    RecoveryExhausted,
    /// A session resume failed because the transcript is unavailable (e.g.
    /// stale session id after a pod restart); the agent fell back to a fresh
    /// session. Previous conversation context is lost. Warning, not an error.
    SessionContextLost,
    /// Lightweight liveness marker published (rate-limited) while a
    /// long-running tool call streams progress. Carries no conversation
    /// content — it exists so the reaper's activity floor keeps advancing
    /// during a multi-minute tool run instead of freezing at the last
    /// message. SSE-only downstream; never persisted as a message.
    Activity,
    /// What the harness said about its own context window: how much of it the
    /// session occupies, and how large it is where the harness states one.
    ContextWindow,
}

impl AgentSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Message => "message",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::PendingResponse => "pending_response",
            Self::SessionInit => "session_init",
            Self::Retrying => "retrying",
            Self::Scheduled => "scheduled",
            Self::RecoveryStarted => "recovery_started",
            Self::RecoveryExhausted => "recovery_exhausted",
            Self::SessionContextLost => "session_context_lost",
            Self::Activity => "activity",
            Self::ContextWindow => "context_window",
        }
    }

    /// Whether this sub-type represents an error condition.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error | Self::RecoveryExhausted)
    }
}

impl std::fmt::Display for AgentSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AgentSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "started" => Ok(Self::Started),
            "stopped" => Ok(Self::Stopped),
            "message" => Ok(Self::Message),
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "pending_response" => Ok(Self::PendingResponse),
            "session_init" => Ok(Self::SessionInit),
            "retrying" => Ok(Self::Retrying),
            "scheduled" => Ok(Self::Scheduled),
            "recovery_started" => Ok(Self::RecoveryStarted),
            "recovery_exhausted" => Ok(Self::RecoveryExhausted),
            "session_context_lost" => Ok(Self::SessionContextLost),
            "activity" => Ok(Self::Activity),
            "context_window" => Ok(Self::ContextWindow),
            other => Err(format!("unknown AgentSubType: {other}")),
        }
    }
}

/// Agent event payload — typed shape for `EventType::Agent` events.
///
/// Agent payloads are pass-through from the sandbox (Claude CLI output).
/// This struct is primarily for OpenAPI schema generation; at runtime the
/// backend continues to forward `serde_json::Value` payloads.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentEventPayload {
    /// The message role type: "assistant", "user", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    /// For "message" subtype: the Claude SDK assistant message.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub message: Option<serde_json::Value>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_key_is_distinct_across_scopes_with_same_id() {
        let user = CodexScope::user("foo").lock_key();
        let global = CodexScope::Global.lock_key();
        assert_ne!(user, global);
        assert!(user.contains("user"));
        assert!(global.contains("global"));
    }

    #[test]
    fn where_clause_per_scope_is_well_formed() {
        let u = CodexScope::user("u1").where_clause("$1");
        assert!(u.contains("user_id = $1"));
        assert!(u.contains("scope = 'user'"));

        let g = CodexScope::Global.where_clause("$1");
        assert!(g.contains("user_id IS NULL"));
        assert!(g.contains("team_id IS NULL"));
        assert!(g.contains("scope = 'global'"));
    }

    #[test]
    fn bind_value_is_the_scope_identifier() {
        assert_eq!(CodexScope::user("alice").bind_value(), "alice");
        assert_eq!(CodexScope::Global.bind_value(), "");
    }

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

/// Agent type identifier (open-ended, e.g. "claude-code").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct AgentType(pub String);

impl AgentType {
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

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for AgentType {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AgentType {
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

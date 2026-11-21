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
    /// Singleton global row: `(user_id IS NULL, scope = 'global')`.
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

    #[test]
    fn agent_profile_wire_form_is_its_column_value() {
        for profile in AgentProfile::ALL {
            let wire = serde_json::to_string(&profile).unwrap();
            assert_eq!(wire, format!("\"{}\"", profile.as_str()));
            assert_eq!(AgentProfile::parse(profile.as_str()), Some(profile));
        }
        assert!(AgentProfile::try_from("claude-kode").is_err());
    }
}

/// The executable family that runs an Agent. The only harness type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    /// Pi over the JSONL RPC transport.
    Pi,
    /// mini-swe-agent over the Python JSONL bridge.
    MiniSwe,
    /// An Agent Client Protocol agent over JSON-RPC on stdio.
    Acp,
}

/// The vendors a Pi model id may name, as `vendor/model`.
///
/// The backend's model gateway reaches each through the deployment's hub or the
/// task's own API key; `openai-codex/…` and the like are refused rather than
/// run against a subscription login.
pub const PI_VENDORS: [&str; 3] = ["anthropic", "openai", "openrouter"];

/// A Pi model id split into its vendor and the model name the gateway is sent,
/// or `None` for a vendor outside [`PI_VENDORS`] or an empty name.
///
/// The name keeps any further `/`: an `openrouter` id is itself `maker/model`.
pub fn split_pi_model(model: &str) -> Option<(&str, &str)> {
    model
        .split_once('/')
        .filter(|(vendor, name)| PI_VENDORS.contains(vendor) && !name.is_empty())
}

impl Harness {
    /// The wire and column value for this harness.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::MiniSwe => "mini-swe",
            Self::Acp => "acp",
        }
    }

    /// Parse a wire or column value; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "mini-swe" => Some(Self::MiniSwe),
            "acp" => Some(Self::Acp),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum AgentProfile {
    ClaudeCode,
    /// Claude Code over the tmux (interactive TUI) transport.
    ClaudeCodeTui,
    /// Claude Code over the subprocess/NDJSON transport.
    ClaudeCodeCli,
    /// Claude Code pinned to the `fable` model.
    ClaudeCodeFable,
    Codex,
    /// Pi over the JSONL RPC transport.
    Pi,
    /// mini-swe-agent over the Python JSONL bridge.
    MiniSwe,
    /// Google's Antigravity CLI (`agy`) over the ACP harness.
    Antigravity,
}

impl AgentProfile {
    /// Every profile, in catalog display order.
    pub const ALL: [Self; 8] = [
        Self::ClaudeCode,
        Self::ClaudeCodeTui,
        Self::ClaudeCodeCli,
        Self::ClaudeCodeFable,
        Self::Codex,
        Self::Pi,
        Self::MiniSwe,
        Self::Antigravity,
    ];

    /// The wire and column value, e.g. `claude-code-tui`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::ClaudeCodeTui => "claude-code-tui",
            Self::ClaudeCodeCli => "claude-code-cli",
            Self::ClaudeCodeFable => "claude-code-fable",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::MiniSwe => "mini-swe",
            Self::Antigravity => "antigravity",
        }
    }

    /// Parse a wire or column value; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }

    /// The harness this profile runs on.
    pub fn harness(self) -> Harness {
        match self {
            Self::ClaudeCode
            | Self::ClaudeCodeTui
            | Self::ClaudeCodeCli
            | Self::ClaudeCodeFable => Harness::ClaudeCode,
            Self::Codex => Harness::Codex,
            Self::Pi => Harness::Pi,
            Self::MiniSwe => Harness::MiniSwe,
            Self::Antigravity => Harness::Acp,
        }
    }
}

impl std::fmt::Display for AgentProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AgentProfile {
    type Err = String;

    /// The `Err` names the valid options, so a caller can surface it verbatim.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| {
            let options = Self::ALL.map(Self::as_str).join(", ");
            format!("unknown agentProfile `{value}`. Available options: {options}")
        })
    }
}

impl TryFrom<String> for AgentProfile {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<&str> for AgentProfile {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
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

crate::string_enum!(
    /// How long Anthropic keeps a Claude Code session's prompt cache warm
    /// between requests. A 1-hour write costs 2× base input against 1.25× for
    /// 5 minutes; reads cost the same.
    ///
    /// arch-allow(floor-type-admission): consumers reach it only through
    /// `AgentProperties`, which is admitted.
    PromptCacheTtl {
        FiveMinutes => "5m",
        OneHour => "1h",
    }
);

/// Harness settings a Loop gives every Task it spawns, and a Task gives every
/// agent session it runs. A property a harness does not read is ignored by it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentProperties {
    /// Claude Code's main-conversation prompt cache TTL. `None` keeps the CLI's
    /// own choice, which is 5 minutes behind the model gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_ttl: Option<PromptCacheTtl>,
}

impl AgentProperties {
    /// The agent environment these properties set, as `(name, value)` pairs.
    pub fn agent_env(&self) -> Vec<(&'static str, &'static str)> {
        let mut env = Vec::new();
        if let Some(ttl) = self.prompt_cache_ttl {
            env.push(("CLAUDE_CODE_PROMPT_CACHE_TTL", ttl.as_str()));
        }
        env
    }
}

#[cfg(test)]
mod agent_properties_tests {
    use super::{AgentProperties, PromptCacheTtl};

    #[test]
    fn prompt_cache_ttl_reads_the_values_the_claude_cli_accepts() {
        let props: AgentProperties = serde_json::from_str(r#"{"promptCacheTtl":"1h"}"#).unwrap();
        assert_eq!(props.prompt_cache_ttl, Some(PromptCacheTtl::OneHour));
        assert_eq!(props.agent_env(), [("CLAUDE_CODE_PROMPT_CACHE_TTL", "1h")]);
        assert!(serde_json::from_str::<AgentProperties>(r#"{"promptCacheTtl":"2h"}"#).is_err());
    }

    #[test]
    fn empty_properties_set_no_env_and_serialize_to_an_empty_object() {
        let props = AgentProperties::default();
        assert!(props.agent_env().is_empty());
        assert_eq!(serde_json::to_string(&props).unwrap(), "{}");
        assert_eq!(
            serde_json::from_str::<AgentProperties>("{}").unwrap(),
            props
        );
    }
}

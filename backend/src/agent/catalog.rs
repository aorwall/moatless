//! Static catalog of supported agents.
//!
//! Adding a new agent is a single-file change: append an entry to [`catalog`].

use crate::sandbox::secrets::{
    ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, OPENAI_API_KEY,
};

/// Static model entry for a catalog agent.
pub struct ModelEntry {
    pub id: &'static str,
    pub label: &'static str,
}

/// Static agent definition.
pub struct AgentEntry {
    pub id: &'static str,
    pub label: &'static str,
    /// Credential keys (secrets / env vars) any of which makes the agent usable.
    pub credential_keys: &'static [&'static str],
    /// Provider-token provider names any of which makes the agent usable.
    pub provider_token_providers: &'static [&'static str],
    pub default_model: &'static str,
    pub models: &'static [ModelEntry],
    /// Hint shown when no credential is available.
    pub missing_credential_hint: &'static str,
}

pub const CLAUDE_CODE: AgentEntry = AgentEntry {
    id: "claude-code",
    label: "Claude Code",
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "default",
    models: &[
        ModelEntry {
            id: "default",
            label: "Default",
        },
        ModelEntry {
            id: "claude-opus-4-6",
            label: "Opus 4.6",
        },
        ModelEntry {
            id: "claude-sonnet-4-6",
            label: "Sonnet 4.6",
        },
        ModelEntry {
            id: "claude-haiku-4-5-20251001",
            label: "Haiku 4.5",
        },
    ],
    missing_credential_hint:
        "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CODEX: AgentEntry = AgentEntry {
    id: "codex",
    label: "Codex",
    credential_keys: &[CODEX_AUTH_JSON, OPENAI_API_KEY],
    provider_token_providers: &["codex", "openai"],
    default_model: "default",
    models: &[ModelEntry {
        id: "default",
        label: "Default",
    }],
    missing_credential_hint: "Add CODEX_AUTH_JSON or OPENAI_API_KEY in Settings \u{2192} Secrets",
};

/// All supported agents in display order.
pub fn catalog() -> &'static [AgentEntry] {
    &[CLAUDE_CODE, CODEX]
}

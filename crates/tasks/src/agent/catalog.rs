//! Static catalog of supported agents.
//!
//! Adding a new agent is a single-file change: append an entry to [`catalog`].

use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, OPENAI_API_KEY,
};
use moatless_types::agent::Harness;

/// Static model entry for a catalog agent.
pub struct ModelEntry {
    pub id: &'static str,
    pub label: &'static str,
}

/// Agent-specific options resolved from the catalog (extensible).
pub struct AgentParams {
    /// Run the Claude Code transport over tmux (interactive TUI) instead of the
    /// subprocess/NDJSON transport.
    pub use_tmux: bool,
}

/// Static agent definition.
pub struct AgentProfileEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub harness: Harness,
    /// Agent-specific options (e.g. `use_tmux`).
    pub params: AgentParams,
    /// Credential keys (secrets / env vars) any of which makes the agent
    /// usable.
    pub credential_keys: &'static [&'static str],
    /// Provider-token provider names any of which makes the agent usable.
    pub provider_token_providers: &'static [&'static str],
    pub default_model: &'static str,
    pub models: &'static [ModelEntry],
    /// Hint shown when no credential is available.
    pub missing_credential_hint: &'static str,
}

pub const CLAUDE_CODE: AgentProfileEntry = AgentProfileEntry {
    id: "claude-code",
    label: "Claude Code",
    harness: Harness::ClaudeCode,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "default",
    models: &[
        ModelEntry {
            id: "default",
            label: "Default",
        },
        ModelEntry {
            id: "opus",
            label: "Claude Opus",
        },
        ModelEntry {
            id: "sonnet",
            label: "Claude Sonnet",
        },
        ModelEntry {
            id: "haiku",
            label: "Claude Haiku",
        },
        ModelEntry {
            id: "fable",
            label: "Claude Fable",
        },
    ],
    missing_credential_hint: "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_TUI: AgentProfileEntry = AgentProfileEntry {
    id: "claude-code-tui",
    label: "Claude Code (TUI)",
    harness: Harness::ClaudeCode,
    params: AgentParams { use_tmux: true },
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "default",
    models: &[
        ModelEntry {
            id: "default",
            label: "Default",
        },
        ModelEntry {
            id: "opus",
            label: "Claude Opus",
        },
        ModelEntry {
            id: "sonnet",
            label: "Claude Sonnet",
        },
        ModelEntry {
            id: "haiku",
            label: "Claude Haiku",
        },
        ModelEntry {
            id: "fable",
            label: "Claude Fable",
        },
    ],
    missing_credential_hint: "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_CLI: AgentProfileEntry = AgentProfileEntry {
    id: "claude-code-cli",
    label: "Claude Code (CLI)",
    harness: Harness::ClaudeCode,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "default",
    models: &[
        ModelEntry {
            id: "default",
            label: "Default",
        },
        ModelEntry {
            id: "opus",
            label: "Claude Opus",
        },
        ModelEntry {
            id: "sonnet",
            label: "Claude Sonnet",
        },
        ModelEntry {
            id: "haiku",
            label: "Claude Haiku",
        },
        ModelEntry {
            id: "fable",
            label: "Claude Fable",
        },
    ],
    missing_credential_hint: "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_FABLE: AgentProfileEntry = AgentProfileEntry {
    id: "claude-code-fable",
    label: "Claude Code (Fable)",
    harness: Harness::ClaudeCode,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "fable",
    models: &[ModelEntry {
        id: "fable",
        label: "Claude Fable",
    }],
    missing_credential_hint: "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CODEX: AgentProfileEntry = AgentProfileEntry {
    id: "codex",
    label: "Codex",
    harness: Harness::Codex,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CODEX_AUTH_JSON, OPENAI_API_KEY],
    provider_token_providers: &["codex", "openai"],
    default_model: "default",
    // "Default" resolves to `DEFAULT_CODEX_MODEL` (`gpt-6-astra`) in the sandbox —
    // see `crates/sandbox/src/agent/codex_app_server/mod.rs`. The explicit entries below mirror
    // the `gpt-*` half of the Codex CLI's own `model/list` at the version pinned in
    // `docker/cli-versions.env`, in the order it returns them. `model/list` also returns the
    // `claude-*` ids Codex can drive; they are deliberately absent here, because a Codex turn on
    // one routes at Anthropic rather than through the OpenAI proxy a pod is configured for.
    models: &[
        ModelEntry {
            id: "default",
            label: "Default",
        },
        ModelEntry {
            id: "gpt-6-astra",
            label: "GPT-6 Astra",
        },
        ModelEntry {
            id: "gpt-6-sol",
            label: "GPT-6 Sol",
        },
        ModelEntry {
            id: "gpt-6-luna",
            label: "GPT-6 Luna",
        },
        ModelEntry {
            id: "gpt-5.6-sol",
            label: "GPT-5.6 Sol",
        },
        ModelEntry {
            id: "gpt-5.6-terra",
            label: "GPT-5.6 Terra",
        },
        ModelEntry {
            id: "gpt-5.6-luna",
            label: "GPT-5.6 Luna",
        },
        ModelEntry {
            id: "gpt-5.5",
            label: "GPT-5.5",
        },
    ],
    missing_credential_hint: "Add CODEX_AUTH_JSON or OPENAI_API_KEY in Settings \u{2192} Secrets",
};

/// All supported agents in display order.
pub fn catalog() -> &'static [AgentProfileEntry] {
    &[
        CLAUDE_CODE,
        CLAUDE_CODE_TUI,
        CLAUDE_CODE_CLI,
        CLAUDE_CODE_FABLE,
        CODEX,
    ]
}

/// Look up a catalog entry by its `agent_profile` id.
pub fn find(id: &str) -> Option<&'static AgentProfileEntry> {
    catalog().iter().find(|e| e.id == id)
}

/// The harness `agent_profile` runs on; `None` for an id the catalog lacks.
pub fn harness_of(agent_profile: &str) -> Option<Harness> {
    find(agent_profile).map(|e| e.harness)
}

/// Whether the agent with this id runs over the tmux transport.
pub fn use_tmux_for(agent_profile: &str) -> bool {
    find(agent_profile).is_some_and(|e| e.params.use_tmux)
}

/// Whether `model` is one of the models `agent_profile` offers.
pub fn offers_model(agent_profile: &str, model: &str) -> bool {
    find(agent_profile).is_some_and(|e| e.models.iter().any(|m| m.id == model))
}

/// Whether `model` is knowably wrong for `agent_profile`: another catalog
/// agent offers it and this one does not.
///
/// `models` is the picker's menu: a pinned upstream id can run unlisted, so
/// only a model another agent owns is rejected on the catalog's word alone.
pub fn model_conflicts(agent_profile: &str, model: &str) -> bool {
    !offers_model(agent_profile, model)
        && catalog()
            .iter()
            .any(|e| e.models.iter().any(|m| m.id == model))
}

/// Check that `agent_profile` names a catalog agent and that `model`, when given,
/// does not belong to a different one.
///
/// The `Err` names the valid options, so a caller can surface it verbatim.
pub fn validate_agent_model(agent_profile: &str, model: Option<&str>) -> Result<(), String> {
    let Some(entry) = find(agent_profile) else {
        let options = catalog()
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "unknown agentProfile `{agent_profile}`. Available options: {options}"
        ));
    };
    let Some(model) = model else {
        return Ok(());
    };
    if !model_conflicts(agent_profile, model) {
        return Ok(());
    }
    let options = entry
        .models
        .iter()
        .map(|m| m.id)
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "model `{model}` belongs to another agentProfile, not `{agent_profile}`. Available options: {options}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{model_conflicts, offers_model, validate_agent_model};

    #[test]
    fn accepts_a_model_the_agent_offers() {
        assert!(validate_agent_model("claude-code", Some("fable")).is_ok());
        assert!(validate_agent_model("claude-code", None).is_ok());
    }

    #[test]
    fn rejects_a_model_from_another_agent() {
        let err =
            validate_agent_model("codex", Some("opus")).expect_err("opus is a claude-code model");
        assert!(
            err.contains("model `opus` belongs to another agentProfile"),
            "{err}"
        );
        assert!(err.contains("gpt-5.6-sol"), "{err}");
    }

    #[test]
    fn rejects_an_unknown_agent() {
        let err = validate_agent_model("claude-kode", None).expect_err("typo is rejected");
        assert!(err.contains("unknown agentProfile `claude-kode`"), "{err}");
    }

    /// `models` is a menu, not an allowlist — the agent CLI takes any model
    /// string, so an unlisted one is unknown rather than wrong.
    #[test]
    fn accepts_a_model_no_agent_offers() {
        assert!(validate_agent_model("claude-code", Some("claude-haiku-4-5-20251001")).is_ok());
        assert!(validate_agent_model("claude-code", Some("fabel")).is_ok());
        assert!(!model_conflicts("claude-code", "fabel"));
        assert!(!offers_model("claude-code", "fabel"));
    }

    #[test]
    fn a_pinned_agent_offers_only_its_own_model() {
        assert!(validate_agent_model("claude-code-fable", Some("fable")).is_ok());
        assert!(validate_agent_model("claude-code-fable", Some("opus")).is_err());
    }
}

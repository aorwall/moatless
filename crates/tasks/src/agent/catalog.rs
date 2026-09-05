//! Static catalog of supported agents.
//!
//! Adding a new agent is a single-file change: append an entry to [`catalog`].

use moatless_core::secret_keys::{
    ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, OPENAI_API_KEY,
};

/// Static model entry for a catalog agent.
pub struct ModelEntry {
    pub id: &'static str,
    pub label: &'static str,
}

/// Which executable family backs an agent.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

/// Agent-specific options resolved from the catalog (extensible).
pub struct AgentParams {
    /// Run the Claude Code transport over tmux (interactive TUI) instead of
    /// the subprocess/NDJSON transport.
    pub use_tmux: bool,
}

/// Static agent definition.
pub struct AgentEntry {
    pub id: &'static str,
    pub label: &'static str,
    /// Which executable family backs this agent.
    pub kind: AgentKind,
    /// Agent-specific options (e.g. `use_tmux`).
    pub params: AgentParams,
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
    kind: AgentKind::ClaudeCode,
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
    missing_credential_hint:
        "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_TUI: AgentEntry = AgentEntry {
    id: "claude-code-tui",
    label: "Claude Code (TUI)",
    kind: AgentKind::ClaudeCode,
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
    missing_credential_hint:
        "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_CLI: AgentEntry = AgentEntry {
    id: "claude-code-cli",
    label: "Claude Code (CLI)",
    kind: AgentKind::ClaudeCode,
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
    missing_credential_hint:
        "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CLAUDE_CODE_FABLE: AgentEntry = AgentEntry {
    id: "claude-code-fable",
    label: "Claude Code (Fable)",
    kind: AgentKind::ClaudeCode,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY],
    provider_token_providers: &["claude"],
    default_model: "fable",
    models: &[ModelEntry {
        id: "fable",
        label: "Claude Fable",
    }],
    missing_credential_hint:
        "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets",
};

pub const CODEX: AgentEntry = AgentEntry {
    id: "codex",
    label: "Codex",
    kind: AgentKind::Codex,
    params: AgentParams { use_tmux: false },
    credential_keys: &[CODEX_AUTH_JSON, OPENAI_API_KEY],
    provider_token_providers: &["codex", "openai"],
    default_model: "default",
    // "Default" resolves to `DEFAULT_CODEX_MODEL` (`gpt-6-astra`) in the sandbox —
    // see `sandbox/src/agent/codex_app_server/mod.rs`. The explicit entries below mirror
    // the Codex CLI's own `model/list` at the version pinned in
    // `docker/cli-versions.env`, in the order it returns them.
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
        ModelEntry {
            id: "gpt-5.4-mini",
            label: "GPT-5.4 Mini",
        },
        ModelEntry {
            id: "gpt-5.3-codex-spark",
            label: "GPT-5.3 Codex Spark",
        },
    ],
    missing_credential_hint: "Add CODEX_AUTH_JSON or OPENAI_API_KEY in Settings \u{2192} Secrets",
};

/// All supported agents in display order.
pub fn catalog() -> &'static [AgentEntry] {
    &[
        CLAUDE_CODE,
        CLAUDE_CODE_TUI,
        CLAUDE_CODE_CLI,
        CLAUDE_CODE_FABLE,
        CODEX,
    ]
}

/// Look up a catalog entry by its `agent_type` id.
pub fn find(id: &str) -> Option<&'static AgentEntry> {
    catalog().iter().find(|e| e.id == id)
}

/// Whether the agent with this id runs over the tmux transport.
pub fn use_tmux_for(agent_type: &str) -> bool {
    find(agent_type).is_some_and(|e| e.params.use_tmux)
}

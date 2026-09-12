/// A catalog model choice that names no model.
///
/// It means *whatever this agent picks*, is resolved inside the Sandbox at run
/// time, and is never written back. Reporting it as a model identity would put
/// a word in the row that names nothing, so the chain falls through it.
pub(crate) const UNRESOLVED_MODEL_CHOICE: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRpcAgentKind {
    ClaudeCode,
    Codex,
}

pub struct UiRpcModelEntry {
    pub id: &'static str,
    pub label: &'static str,
}

pub struct UiRpcAgentEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: UiRpcAgentKind,
    pub default_model: &'static str,
    pub models: &'static [UiRpcModelEntry],
    pub missing_credential_hint: &'static str,
}

pub const CLAUDE_CODE: UiRpcAgentEntry = UiRpcAgentEntry {
    id: "claude-code",
    label: "Claude Code",
    kind: UiRpcAgentKind::ClaudeCode,
    default_model: UNRESOLVED_MODEL_CHOICE,
    missing_credential_hint: "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings > Secrets",
    models: &[
        UiRpcModelEntry {
            id: UNRESOLVED_MODEL_CHOICE,
            label: "Default",
        },
        UiRpcModelEntry {
            id: "opus",
            label: "Claude Opus",
        },
        UiRpcModelEntry {
            id: "sonnet",
            label: "Claude Sonnet",
        },
        UiRpcModelEntry {
            id: "haiku",
            label: "Claude Haiku",
        },
        UiRpcModelEntry {
            id: "fable",
            label: "Claude Fable",
        },
    ],
};

pub const CLAUDE_CODE_TUI: UiRpcAgentEntry = UiRpcAgentEntry {
    id: "claude-code-tui",
    label: "Claude Code (TUI)",
    kind: UiRpcAgentKind::ClaudeCode,
    default_model: UNRESOLVED_MODEL_CHOICE,
    missing_credential_hint: CLAUDE_CODE.missing_credential_hint,
    models: CLAUDE_CODE.models,
};

pub const CLAUDE_CODE_CLI: UiRpcAgentEntry = UiRpcAgentEntry {
    id: "claude-code-cli",
    label: "Claude Code (CLI)",
    kind: UiRpcAgentKind::ClaudeCode,
    default_model: UNRESOLVED_MODEL_CHOICE,
    missing_credential_hint: CLAUDE_CODE.missing_credential_hint,
    models: CLAUDE_CODE.models,
};

pub const CLAUDE_CODE_FABLE: UiRpcAgentEntry = UiRpcAgentEntry {
    id: "claude-code-fable",
    label: "Claude Code (Fable)",
    kind: UiRpcAgentKind::ClaudeCode,
    default_model: "fable",
    missing_credential_hint: CLAUDE_CODE.missing_credential_hint,
    models: &[UiRpcModelEntry {
        id: "fable",
        label: "Claude Fable",
    }],
};

pub const CODEX: UiRpcAgentEntry = UiRpcAgentEntry {
    id: "codex",
    label: "Codex",
    kind: UiRpcAgentKind::Codex,
    default_model: UNRESOLVED_MODEL_CHOICE,
    missing_credential_hint: "Add CODEX_AUTH_JSON or OPENAI_API_KEY in Settings > Secrets",
    models: &[
        UiRpcModelEntry {
            id: UNRESOLVED_MODEL_CHOICE,
            label: "Default",
        },
        UiRpcModelEntry {
            id: "gpt-6-astra",
            label: "GPT-6 Astra",
        },
        UiRpcModelEntry {
            id: "gpt-5.6-sol",
            label: "GPT-5.6 Sol",
        },
        UiRpcModelEntry {
            id: "gpt-5.6-terra",
            label: "GPT-5.6 Terra",
        },
        UiRpcModelEntry {
            id: "gpt-5.6-luna",
            label: "GPT-5.6 Luna",
        },
        UiRpcModelEntry {
            id: "gpt-5.5",
            label: "GPT-5.5",
        },
        UiRpcModelEntry {
            id: "gpt-5.3-codex-spark",
            label: "GPT-5.3 Codex Spark",
        },
    ],
};

pub fn catalog() -> &'static [UiRpcAgentEntry] {
    &[
        CLAUDE_CODE,
        CLAUDE_CODE_TUI,
        CLAUDE_CODE_CLI,
        CLAUDE_CODE_FABLE,
        CODEX,
    ]
}

pub fn find(id: &str) -> Option<&'static UiRpcAgentEntry> {
    catalog().iter().find(|entry| entry.id == id)
}

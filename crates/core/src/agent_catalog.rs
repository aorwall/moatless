//! Static catalog of supported agents, one entry per [`AgentProfile`].

use crate::secret_keys::{
    ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON, OPENAI_API_KEY,
};
use moatless_types::agent::{
    AgentProfile, Harness, MINI_SWE_VENDORS, PI_VENDORS, split_vendored_model,
};
use moatless_types::model::{ClaudeModel, CodexModel};

/// A catalog model choice that names no model: the sandbox runs the CLI's own
/// default for Claude, and the backend resolves Codex's before delivery.
///
/// Never report it as a model identity: it names nothing.
pub const UNRESOLVED_MODEL_CHOICE: &str = "default";

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

/// Which model ids an agent runs beyond its static `models`, and where the
/// hub's listing of them comes from.
#[derive(Clone, Copy)]
pub enum ModelSource {
    /// Only its static `models`.
    Catalog,
    /// Any `vendor/model` id of these vendors, through the model gateway; the
    /// hub lists the chat models its rows of these vendors serve.
    Vendored(&'static [&'static str]),
    /// The hub's own bare ids, from the rows this owner serves.
    HubOwner(&'static str),
}

/// Static agent definition.
pub struct AgentProfileEntry {
    pub profile: AgentProfile,
    pub label: &'static str,
    /// Agent-specific options (e.g. `use_tmux`).
    pub params: AgentParams,
    /// Credential keys (secrets / env vars) any of which makes the agent
    /// usable.
    pub credential_keys: &'static [&'static str],
    /// Provider-token provider names any of which makes the agent usable.
    pub provider_token_providers: &'static [&'static str],
    pub default_model: &'static str,
    pub models: &'static [ModelEntry],
    pub model_source: ModelSource,
    /// Hint shown when no credential is available.
    pub missing_credential_hint: &'static str,
}

impl AgentProfileEntry {
    /// The `agent_profile` wire id.
    pub fn id(&self) -> &'static str {
        self.profile.as_str()
    }

    pub fn harness(&self) -> Harness {
        self.profile.harness()
    }
}

const CLAUDE_CREDENTIAL_KEYS: &[&str] = &[CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY];
const CLAUDE_MISSING_CREDENTIAL_HINT: &str =
    "Add CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY in Settings \u{2192} Secrets";
const DEFAULT_MODEL: ModelEntry = ModelEntry {
    id: UNRESOLVED_MODEL_CHOICE,
    label: "Default",
};

const fn claude_model(model: ClaudeModel) -> ModelEntry {
    ModelEntry {
        id: model.alias(),
        label: model.label(),
    }
}

const fn codex_model(model: CodexModel) -> ModelEntry {
    ModelEntry {
        id: model.id(),
        label: model.label(),
    }
}

const FABLE: ModelEntry = claude_model(ClaudeModel::Fable);
const CLAUDE_MODELS: &[ModelEntry] = &[
    DEFAULT_MODEL,
    claude_model(ClaudeModel::Opus),
    claude_model(ClaudeModel::Sonnet),
    claude_model(ClaudeModel::Haiku),
    FABLE,
];

const fn claude_code(
    profile: AgentProfile,
    label: &'static str,
    use_tmux: bool,
) -> AgentProfileEntry {
    AgentProfileEntry {
        profile,
        label,
        params: AgentParams { use_tmux },
        credential_keys: CLAUDE_CREDENTIAL_KEYS,
        provider_token_providers: &["claude"],
        default_model: UNRESOLVED_MODEL_CHOICE,
        models: CLAUDE_MODELS,
        model_source: ModelSource::Catalog,
        missing_credential_hint: CLAUDE_MISSING_CREDENTIAL_HINT,
    }
}

pub const CLAUDE_CODE: AgentProfileEntry =
    claude_code(AgentProfile::ClaudeCode, "Claude Code", false);
pub const CLAUDE_CODE_TUI: AgentProfileEntry =
    claude_code(AgentProfile::ClaudeCodeTui, "Claude Code (TUI)", true);
pub const CLAUDE_CODE_CLI: AgentProfileEntry =
    claude_code(AgentProfile::ClaudeCodeCli, "Claude Code (CLI)", false);

pub const CLAUDE_CODE_FABLE: AgentProfileEntry = AgentProfileEntry {
    default_model: FABLE.id,
    models: &[FABLE],
    ..claude_code(AgentProfile::ClaudeCodeFable, "Claude Code (Fable)", false)
};

pub const CODEX: AgentProfileEntry = AgentProfileEntry {
    profile: AgentProfile::Codex,
    label: "Codex",
    params: AgentParams { use_tmux: false },
    credential_keys: &[CODEX_AUTH_JSON, OPENAI_API_KEY],
    provider_token_providers: &["codex", "openai"],
    default_model: UNRESOLVED_MODEL_CHOICE,
    // Mirrors the `gpt-*` half of the Codex CLI's `model/list` (pinned in
    // `docker/cli-versions.env`), in order. `claude-*` ids are deliberately
    // excluded: a Codex turn on one routes directly to Anthropic, bypassing
    // the OpenAI proxy a pod is configured for.
    models: &[
        DEFAULT_MODEL,
        codex_model(CodexModel::Gpt62Sol),
        codex_model(CodexModel::Gpt62Astra),
        codex_model(CodexModel::Gpt61Terra),
        codex_model(CodexModel::Gpt61Sol),
        codex_model(CodexModel::Gpt6Astra),
        codex_model(CodexModel::Gpt6Sol),
        codex_model(CodexModel::Gpt6Luna),
        codex_model(CodexModel::Gpt56Sol),
        codex_model(CodexModel::Gpt56Terra),
        codex_model(CodexModel::Gpt56Luna),
        codex_model(CodexModel::Gpt55),
    ],
    model_source: ModelSource::Catalog,
    missing_credential_hint: "Add CODEX_AUTH_JSON or OPENAI_API_KEY in Settings \u{2192} Secrets",
};

/// Pi models routed through the backend's API-key gateway.
pub const PI: AgentProfileEntry = AgentProfileEntry {
    profile: AgentProfile::Pi,
    label: "Pi",
    params: AgentParams { use_tmux: false },
    credential_keys: &[ANTHROPIC_API_KEY, OPENAI_API_KEY],
    provider_token_providers: &[],
    default_model: "anthropic/claude-sonnet-5-5",
    models: &[
        ModelEntry {
            id: "anthropic/claude-sonnet-5-5",
            label: "Sonnet",
        },
        ModelEntry {
            id: "anthropic/claude-haiku-5-5",
            label: "Haiku",
        },
        ModelEntry {
            id: "anthropic/claude-opus-5-5",
            label: "Opus",
        },
        ModelEntry {
            id: "openai/gpt-6.1-sol",
            label: "GPT 6.1 Sol",
        },
    ],
    model_source: ModelSource::Vendored(PI_VENDORS),
    missing_credential_hint: "Add ANTHROPIC_API_KEY or OPENAI_API_KEY in Settings → Secrets",
};

/// mini-swe-agent models routed through the model gateway.
pub const MINI_SWE: AgentProfileEntry = AgentProfileEntry {
    profile: AgentProfile::MiniSwe,
    label: "mini-swe-agent",
    params: AgentParams { use_tmux: false },
    credential_keys: &[ANTHROPIC_API_KEY, OPENAI_API_KEY],
    provider_token_providers: &[],
    default_model: "openai/gpt-6.1-sol",
    models: PI.models,
    model_source: ModelSource::Vendored(MINI_SWE_VENDORS),
    missing_credential_hint: "Add ANTHROPIC_API_KEY or OPENAI_API_KEY, or configure the model hub",
};

/// Antigravity (`agy`) models, as the deployment's hub names them: the hub's
/// `antigravity` rows, which are appended at runtime, follow this default.
pub const ANTIGRAVITY: AgentProfileEntry = AgentProfileEntry {
    profile: AgentProfile::Antigravity,
    label: "Antigravity",
    params: AgentParams { use_tmux: false },
    credential_keys: &[OPENAI_API_KEY],
    provider_token_providers: &[],
    default_model: "gemini-3-flash",
    models: &[ModelEntry {
        id: "gemini-3-flash",
        label: "Gemini 3 Flash",
    }],
    model_source: ModelSource::HubOwner("antigravity"),
    missing_credential_hint: "Configure the model hub, or add OPENAI_API_KEY in Settings \u{2192} Secrets",
};

/// The catalog entry for `profile`.
pub fn entry(profile: AgentProfile) -> &'static AgentProfileEntry {
    match profile {
        AgentProfile::ClaudeCode => &CLAUDE_CODE,
        AgentProfile::ClaudeCodeTui => &CLAUDE_CODE_TUI,
        AgentProfile::ClaudeCodeCli => &CLAUDE_CODE_CLI,
        AgentProfile::ClaudeCodeFable => &CLAUDE_CODE_FABLE,
        AgentProfile::Codex => &CODEX,
        AgentProfile::Pi => &PI,
        AgentProfile::MiniSwe => &MINI_SWE,
        AgentProfile::Antigravity => &ANTIGRAVITY,
    }
}

/// All supported agents in display order.
pub fn catalog() -> impl Iterator<Item = &'static AgentProfileEntry> {
    AgentProfile::ALL.into_iter().map(entry)
}

/// Look up a catalog entry by its `agent_profile` id.
pub fn find(id: &str) -> Option<&'static AgentProfileEntry> {
    AgentProfile::parse(id).map(entry)
}

/// Whether the agent with this id runs over the tmux transport.
pub fn use_tmux_for(agent_profile: AgentProfile) -> bool {
    entry(agent_profile).params.use_tmux
}

/// Whether `model` is one of the models `agent_profile` offers.
pub fn offers_model(agent_profile: AgentProfile, model: &str) -> bool {
    entry(agent_profile).models.iter().any(|m| m.id == model)
}

/// Whether the model hub supplies this agent's models, so its `models` list
/// is only the part known without asking the hub.
pub fn lists_hub_models(agent_profile: AgentProfile) -> bool {
    !matches!(entry(agent_profile).model_source, ModelSource::Catalog)
}

/// Whether a requested `model` may run under `agent_profile`: one it lists, or,
/// for an agent whose models come from the hub, one no other agent owns.
pub fn accepts_model(agent_profile: AgentProfile, model: &str) -> bool {
    offers_model(agent_profile, model)
        || (lists_hub_models(agent_profile) && !model_conflicts(agent_profile, model))
}

/// Whether `model` is knowably wrong for `agent_profile`: another catalog
/// agent offers it and this one does not.
///
/// `models` is the picker's menu: a pinned upstream id can run unlisted, so
/// only a model another agent owns is rejected on the catalog's word alone.
pub fn model_conflicts(agent_profile: AgentProfile, model: &str) -> bool {
    !offers_model(agent_profile, model) && catalog().any(|e| e.models.iter().any(|m| m.id == model))
}

/// Check that `model`, when given, does not belong to a different agent than
/// `agent_profile`.
///
/// The `Err` names the valid options, so a caller can surface it verbatim.
pub fn validate_agent_model(
    agent_profile: AgentProfile,
    model: Option<&str>,
) -> Result<(), String> {
    let Some(model) = model else {
        return Ok(());
    };
    let agent = entry(agent_profile);
    match agent.model_source {
        ModelSource::Catalog => {}
        ModelSource::Vendored(vendors) => {
            if split_vendored_model(vendors, model).is_none() {
                let forms: Vec<String> = vendors.iter().map(|v| format!("{v}/model")).collect();
                let forms = match forms.split_last() {
                    Some((last, [])) => last.clone(),
                    Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
                    None => String::new(),
                };
                return Err(format!(
                    "{} requires {forms} using an API key or hub",
                    agent.label
                ));
            }
        }
        ModelSource::HubOwner(_) => {
            if model.contains('/') {
                return Err(format!(
                    "{} takes the hub's own model id, such as {}",
                    agent.label, agent.default_model
                ));
            }
        }
    }
    if !model_conflicts(agent_profile, model) {
        return Ok(());
    }
    let options = agent
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
    use super::{
        UNRESOLVED_MODEL_CHOICE, entry, model_conflicts, offers_model, validate_agent_model,
    };
    use moatless_types::agent::AgentProfile;
    use moatless_types::model::{ClaudeModel, CodexModel};

    #[test]
    fn pi_refuses_subscription_providers_and_unqualified_models() {
        for model in [
            "openai-codex/gpt-6.1-sol",
            "claude-sonnet-5-5",
            "anthropic/",
        ] {
            assert!(validate_agent_model(AgentProfile::Pi, Some(model)).is_err());
        }
        assert!(validate_agent_model(AgentProfile::Pi, Some("openai/future-model")).is_ok());
        assert!(
            validate_agent_model(AgentProfile::Pi, Some("openrouter/mistralai/mistral-large"))
                .is_ok()
        );
        assert!(validate_agent_model(AgentProfile::Pi, Some("openrouter/")).is_err());
    }

    #[test]
    fn mini_swe_accepts_only_tested_api_key_providers() {
        for id in ["openai/gpt-6.1-sol", "anthropic/claude-sonnet-5-5"] {
            assert!(validate_agent_model(AgentProfile::MiniSwe, Some(id)).is_ok());
        }
        for id in [
            "openai-codex/gpt-6.1-sol",
            "openrouter/test",
            "openai/",
            "sonnet",
        ] {
            assert!(validate_agent_model(AgentProfile::MiniSwe, Some(id)).is_err());
        }
    }

    fn offered(profile: AgentProfile) -> Vec<&'static str> {
        entry(profile)
            .models
            .iter()
            .map(|m| m.id)
            .filter(|id| *id != UNRESOLVED_MODEL_CHOICE)
            .collect()
    }

    #[test]
    fn every_model_is_offered_in_order() {
        let claude: Vec<_> = ClaudeModel::ALL.map(ClaudeModel::alias).into();
        let codex: Vec<_> = CodexModel::ALL.map(CodexModel::id).into();
        assert_eq!(offered(AgentProfile::ClaudeCode), claude);
        assert_eq!(offered(AgentProfile::Codex), codex);
    }

    #[test]
    fn accepts_a_model_the_agent_offers() {
        assert!(validate_agent_model(AgentProfile::ClaudeCode, Some("fable")).is_ok());
        assert!(validate_agent_model(AgentProfile::ClaudeCode, None).is_ok());
    }

    #[test]
    fn rejects_a_model_from_another_agent() {
        let err = validate_agent_model(AgentProfile::Codex, Some("opus"))
            .expect_err("opus is a claude-code model");
        assert!(
            err.contains("model `opus` belongs to another agentProfile"),
            "{err}"
        );
        assert!(err.contains("gpt-5.6-sol"), "{err}");
    }

    #[test]
    fn rejects_an_unknown_agent() {
        let err = "claude-kode"
            .parse::<AgentProfile>()
            .expect_err("typo is rejected");
        assert!(err.contains("unknown agentProfile `claude-kode`"), "{err}");
    }

    /// `models` is a menu, not an allowlist — the agent CLI takes any model
    /// string, so an unlisted one is unknown rather than wrong.
    #[test]
    fn accepts_a_model_no_agent_offers() {
        let claude = AgentProfile::ClaudeCode;
        assert!(validate_agent_model(claude, Some("claude-haiku-4-5-20251001")).is_ok());
        assert!(validate_agent_model(claude, Some("fabel")).is_ok());
        assert!(!model_conflicts(claude, "fabel"));
        assert!(!offers_model(claude, "fabel"));
    }

    #[test]
    fn a_pinned_agent_offers_only_its_own_model() {
        let fable = AgentProfile::ClaudeCodeFable;
        assert!(validate_agent_model(fable, Some("fable")).is_ok());
        assert!(validate_agent_model(fable, Some("opus")).is_err());
    }

    /// Every hub-listed agent's refusal names its own vendors, from its
    /// `model_source`.
    #[test]
    fn a_vendored_agent_names_its_vendors_when_it_refuses_a_model() {
        let pi = validate_agent_model(AgentProfile::Pi, Some("gemini-3-flash")).unwrap_err();
        assert_eq!(
            pi,
            "Pi requires anthropic/model, openai/model or openrouter/model using an API key or hub"
        );
        let swe = validate_agent_model(AgentProfile::MiniSwe, Some("openrouter/x")).unwrap_err();
        assert_eq!(
            swe,
            "mini-swe-agent requires anthropic/model or openai/model using an API key or hub"
        );
    }
}

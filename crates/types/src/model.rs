//! The models the catalog offers, and what each one is: id, label, price and
//! the effort levels it accepts.

/// The effort levels the Claude CLI's `--effort` accepts, weakest first. It
/// warns about and ignores anything else, and does not vary them per model.
pub const CLAUDE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The effort suffixes the Antigravity hub's catalog puts on a model id, as in
/// `gemini-4-neon-xhigh`. `agy` may drop one before it calls the gateway.
pub const HUB_EFFORT_SUFFIXES: &[&str] = &["-low", "-medium", "-high", "-xhigh"];

/// `id` without its [`HUB_EFFORT_SUFFIXES`] suffix, or `id` when it has none.
pub fn strip_hub_effort_suffix(id: &str) -> &str {
    HUB_EFFORT_SUFFIXES
        .iter()
        .find_map(|suffix| id.strip_suffix(suffix))
        .unwrap_or(id)
}

/// A Claude model the catalog offers, named by its catalog alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaudeModel {
    Opus,
    Sonnet,
    Haiku,
    Fable,
}

impl ClaudeModel {
    /// Every Claude model, in catalog order.
    pub const ALL: [Self; 4] = [Self::Opus, Self::Sonnet, Self::Haiku, Self::Fable];

    /// The catalog alias, which is what a task or message stores.
    pub const fn alias(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Fable => "fable",
        }
    }

    /// The Anthropic model id the Sandbox passes to the Claude CLI, since a
    /// proxy routing on the id answers an alias with `unknown provider for
    /// model opus`.
    pub const fn api_id(self) -> &'static str {
        match self {
            Self::Opus => "claude-opus-5-5",
            Self::Sonnet => "claude-sonnet-5-5",
            Self::Haiku => "claude-haiku-5-5",
            Self::Fable => "claude-fable-5-2",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Opus => "Claude Opus 5.5",
            Self::Sonnet => "Claude Sonnet 5.5",
            Self::Haiku => "Claude Haiku 5.5",
            Self::Fable => "Claude Fable 5.2",
        }
    }

    /// Haiku at its rates for prompts up to 100K tokens, which apply
    /// throughout; a longer prompt costs five times as much.
    pub const fn price(self) -> ModelPrice {
        match self {
            Self::Opus => ModelPrice::new(4.0, 5.0, 0.2, 20.0),
            Self::Sonnet => ModelPrice::new(2.0, 2.5, 0.1, 10.0),
            Self::Haiku => ModelPrice::new(0.1, 0.125, 0.01, 0.5),
            Self::Fable => ModelPrice::new(8.0, 10.0, 0.2, 40.0),
        }
    }

    pub fn from_alias(alias: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.alias() == alias)
    }
}

/// A Codex model the catalog offers, in the order the Codex CLI's
/// `model/list` returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodexModel {
    Gpt62Sol,
    Gpt62Astra,
    Gpt61Terra,
    Gpt61Sol,
    Gpt6Astra,
    Gpt6Sol,
    Gpt6Luna,
    Gpt56Sol,
    Gpt56Terra,
    Gpt56Luna,
    Gpt55,
}

/// What every Codex model accepts, weakest first.
const CODEX_BASE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh"];
const CODEX_MAX_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_ULTRA_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

impl CodexModel {
    /// Every Codex model, in catalog order.
    pub const ALL: [Self; 11] = [
        Self::Gpt62Sol,
        Self::Gpt62Astra,
        Self::Gpt61Terra,
        Self::Gpt61Sol,
        Self::Gpt6Astra,
        Self::Gpt6Sol,
        Self::Gpt6Luna,
        Self::Gpt56Sol,
        Self::Gpt56Terra,
        Self::Gpt56Luna,
        Self::Gpt55,
    ];

    /// The model id, the same in the catalog, on the wire and to the CLI.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Gpt62Sol => "gpt-6.2-sol",
            Self::Gpt62Astra => "gpt-6.2-astra",
            Self::Gpt61Sol => "gpt-6.1-sol",
            Self::Gpt6Astra => "gpt-6-astra",
            Self::Gpt6Sol => "gpt-6-sol",
            Self::Gpt6Luna => "gpt-6-luna",
            Self::Gpt56Sol => "gpt-5.6-sol",
            Self::Gpt61Terra => "gpt-6.1-terra",
            Self::Gpt56Terra => "gpt-5.6-terra",
            Self::Gpt56Luna => "gpt-5.6-luna",
            Self::Gpt55 => "gpt-5.5",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Gpt62Sol => "GPT-6.2 Sol",
            Self::Gpt62Astra => "GPT-6.2 Astra",
            Self::Gpt61Sol => "GPT-6.1 Sol",
            Self::Gpt6Astra => "GPT-6 Astra",
            Self::Gpt6Sol => "GPT-6 Sol",
            Self::Gpt6Luna => "GPT-6 Luna",
            Self::Gpt56Sol => "GPT-5.6 Sol",
            Self::Gpt61Terra => "GPT-6.1 Terra",
            Self::Gpt56Terra => "GPT-5.6 Terra",
            Self::Gpt56Luna => "GPT-5.6 Luna",
            Self::Gpt55 => "GPT-5.5",
        }
    }

    /// The effort levels the model accepts, weakest first. Codex fails the
    /// whole turn on a level its model rejects rather than falling back.
    pub const fn effort_levels(self) -> &'static [&'static str] {
        match self {
            Self::Gpt62Sol
            | Self::Gpt62Astra
            | Self::Gpt61Sol
            | Self::Gpt6Astra
            | Self::Gpt6Sol
            | Self::Gpt56Sol
            | Self::Gpt61Terra
            | Self::Gpt56Terra => CODEX_ULTRA_EFFORT_LEVELS,
            Self::Gpt6Luna | Self::Gpt56Luna => CODEX_MAX_EFFORT_LEVELS,
            Self::Gpt55 => CODEX_BASE_EFFORT_LEVELS,
        }
    }

    /// The GPT-6.2, GPT-6.1, GPT-6 and GPT-5.6 families at their short-context
    /// (<272K input) rates, which apply throughout.
    pub const fn price(self) -> ModelPrice {
        match self {
            // Cached reads at a twentieth of input rather than `tiered`'s tenth.
            Self::Gpt62Sol => ModelPrice::new(2.0, 2.5, 0.1, 10.0),
            Self::Gpt62Astra => ModelPrice::new(8.0, 10.0, 0.4, 40.0),
            Self::Gpt61Sol => ModelPrice::new(2.0, 2.5, 0.1, 10.0),
            Self::Gpt61Terra => ModelPrice::new(1.0, 1.25, 0.05, 5.0),
            Self::Gpt6Astra => ModelPrice::tiered(10.0, 50.0),
            Self::Gpt6Sol => ModelPrice::tiered(2.0, 10.0),
            Self::Gpt6Luna => ModelPrice::tiered(0.1, 0.5),
            Self::Gpt56Sol => ModelPrice::tiered(4.0, 20.0),
            Self::Gpt56Terra => ModelPrice::tiered(2.0, 12.0),
            Self::Gpt56Luna => ModelPrice::tiered(0.2, 1.2),
            Self::Gpt55 => EARLIER_CODEX,
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// The effort levels a Codex model id accepts; an id the catalog does not
/// offer gets the levels every Codex model accepts.
pub fn codex_effort_levels(model: &str) -> &'static [&'static str] {
    CodexModel::parse(model).map_or(CODEX_BASE_EFFORT_LEVELS, CodexModel::effort_levels)
}

/// The Anthropic model id a Claude catalog alias names, or `None` for anything
/// that is not an alias: an id already, or a choice naming no model.
pub fn claude_model_id(alias: &str) -> Option<&'static str> {
    ClaudeModel::from_alias(alias).map(ClaudeModel::api_id)
}

/// Per-1M-token USD prices for one model's tiers. `cache_write` prices the
/// cache-creation input at the 5m-TTL tier; `cache_read` the cached reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl ModelPrice {
    const fn new(input: f64, cache_write: f64, cache_read: f64, output: f64) -> Self {
        Self {
            input,
            cache_write,
            cache_read,
            output,
        }
    }

    const fn tiered(input: f64, output: f64) -> Self {
        Self::new(input, input * 1.25, input / 10.0, output)
    }
}

/// Earlier Claude ids no catalog model names, most specific first:
/// `sonnet-5` must precede `sonnet`, and `fable-5-1` `fable`, as each contains the next.
const EARLIER_CLAUDE: &[(&str, ModelPrice)] = &[
    ("sonnet-5", ModelPrice::new(2.0, 2.5, 0.2, 10.0)),
    ("sonnet", ModelPrice::new(3.0, 3.75, 0.3, 15.0)),
    ("opus", ModelPrice::new(5.0, 6.25, 0.5, 25.0)),
    ("haiku", ModelPrice::new(1.0, 1.25, 0.1, 5.0)),
    ("fable-5-1", ModelPrice::new(10.0, 12.5, 0.25, 50.0)),
    ("fable", ModelPrice::new(10.0, 12.5, 1.0, 50.0)),
];
/// Codex ids before the GPT-5.6 family. Cache-write is not a tier there (the
/// cache-creation count is always 0), so it rides the input rate.
const EARLIER_CODEX: ModelPrice = ModelPrice::new(1.25, 1.25, 0.125, 10.0);

/// Models only the Antigravity hub serves, at Google's paid-tier list prices
/// (Vertex AI's for `gpt-oss`). Their caching is implicit, so cache-write
/// rides the input rate; `gpt-oss` has no cached rate.
const ANTIGRAVITY_ONLY: &[(&str, ModelPrice)] = &[
    ("gemini-4-neon", ModelPrice::new(3.0, 3.0, 0.15, 15.0)),
    ("gemini-4-argon", ModelPrice::new(2.0, 2.0, 0.1, 10.0)),
    ("gemini-4-hydrogen", ModelPrice::new(1.5, 1.5, 0.075, 7.5)),
    ("gemini-3.8-flash", ModelPrice::new(0.75, 0.75, 0.075, 3.75)),
    (
        "gemini-3.5-flash-lite",
        ModelPrice::new(0.3, 0.3, 0.03, 2.5),
    ),
    (
        "gemini-3.1-flash-lite",
        ModelPrice::new(0.25, 0.25, 0.025, 1.5),
    ),
    ("gemini-3-flash", ModelPrice::new(0.5, 0.5, 0.05, 3.0)),
    ("gpt-oss-120b", ModelPrice::new(0.09, 0.09, 0.09, 0.36)),
];

/// Prices for a reported model id, or `None` for an unrecognised model.
///
/// A catalog model or one in [`ANTIGRAVITY_ONLY`] matches on its id anywhere
/// in the reported one, and an earlier id on its family in [`EARLIER_CLAUDE`]
/// or as an earlier Codex id.
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let m = model.to_ascii_lowercase();
    if let Some(codex) = CodexModel::ALL.into_iter().find(|c| m.contains(c.id())) {
        return Some(codex.price());
    }
    if let Some(claude) = ClaudeModel::ALL
        .into_iter()
        .find(|c| m.contains(c.api_id()) || m == c.alias())
    {
        return Some(claude.price());
    }
    if let Some((_, price)) = ANTIGRAVITY_ONLY.iter().find(|(id, _)| m.contains(id)) {
        return Some(*price);
    }
    if m.contains("codex") || m.contains("gpt-5") {
        return Some(EARLIER_CODEX);
    }
    EARLIER_CLAUDE
        .iter()
        .find(|(family, _)| m.contains(family))
        .map(|(_, price)| *price)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The price of an earlier family entry, by its key.
    fn earlier(family: &str) -> Option<ModelPrice> {
        EARLIER_CLAUDE
            .iter()
            .find(|(f, _)| *f == family)
            .map(|(_, price)| *price)
    }

    #[test]
    fn a_catalog_model_takes_its_own_price() {
        assert_eq!(
            price_for("claude-sonnet-5-5"),
            Some(ClaudeModel::Sonnet.price())
        );
        assert_eq!(price_for("haiku"), Some(ClaudeModel::Haiku.price()));
        assert_eq!(
            price_for("claude-fable-5-2[1m]"),
            Some(ClaudeModel::Fable.price())
        );
        assert_eq!(price_for("gpt-5.6-sol"), Some(CodexModel::Gpt56Sol.price()));
    }

    #[test]
    fn an_earlier_generation_takes_its_familys_earlier_price() {
        assert_eq!(price_for("claude-sonnet-5"), earlier("sonnet-5"));
        assert_eq!(price_for("claude-sonnet-4-6"), earlier("sonnet"));
        assert_eq!(price_for("claude-opus-5"), earlier("opus"));
        assert_eq!(price_for("claude-opus-4-8"), earlier("opus"));
        assert_eq!(price_for("claude-haiku-4-5-20251001"), earlier("haiku"));
        assert_eq!(price_for("claude-fable-5-1"), earlier("fable-5-1"));
        assert_eq!(price_for("claude-fable-5"), earlier("fable"));
        assert_eq!(price_for("gpt-5-codex"), Some(EARLIER_CODEX));
        assert_eq!(price_for("gpt-6-nova"), None);
    }

    /// The ids the Antigravity hub account serves, as the run reports them.
    #[test]
    fn every_antigravity_model_has_a_price() {
        for id in [
            "gemini-3-flash",
            "gemini-3.1-flash-lite",
            "gemini-3.5-flash-lite",
            "gemini-3.8-flash-high",
            "gemini-3.8-flash-tiered",
            "gemini-4-argon-high",
            "gemini-4-hydrogen-xhigh",
            "gemini-4-neon-xhigh",
            "gpt-oss-120b-medium",
            "claude-sonnet-5-5-high",
            "claude-opus-5-5-high",
        ] {
            assert!(price_for(id).is_some(), "{id} is unpriced");
        }
        assert_eq!(
            price_for("gemini-4-neon-xhigh"),
            Some(ModelPrice::new(3.0, 3.0, 0.15, 15.0))
        );
        assert_eq!(
            price_for("claude-opus-5-5-high"),
            Some(ClaudeModel::Opus.price())
        );
    }

    #[test]
    fn a_hub_id_loses_only_its_trailing_effort_suffix() {
        assert_eq!(
            strip_hub_effort_suffix("gemini-4-neon-xhigh"),
            "gemini-4-neon"
        );
        assert_eq!(
            strip_hub_effort_suffix("claude-opus-5-5-high"),
            "claude-opus-5-5"
        );
        assert_eq!(strip_hub_effort_suffix("gemini-3-flash"), "gemini-3-flash");
    }

    #[test]
    fn an_unlisted_codex_model_accepts_only_the_base_levels() {
        assert_eq!(codex_effort_levels("gpt-6-nova"), CODEX_BASE_EFFORT_LEVELS);
        assert!(codex_effort_levels("gpt-6-astra").contains(&"ultra"));
    }
}

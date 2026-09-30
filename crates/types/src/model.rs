//! The models the catalog offers, and what each one is: id, label, price and
//! the effort levels it accepts.

/// The effort levels the Claude CLI's `--effort` accepts, weakest first. It
/// warns about and ignores anything else, and does not vary them per model.
pub const CLAUDE_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

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
            Self::Sonnet => "claude-sonnet-5",
            Self::Haiku => "claude-haiku-4-5-20251001",
            Self::Fable => "claude-fable-5-1",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Opus => "Claude Opus 5.5",
            Self::Sonnet => "Claude Sonnet 5",
            Self::Haiku => "Claude Haiku 4.5",
            Self::Fable => "Claude Fable 5.1",
        }
    }

    pub const fn price(self) -> ModelPrice {
        match self {
            Self::Opus => ModelPrice::new(5.0, 6.25, 0.5, 25.0),
            Self::Sonnet => ModelPrice::new(2.0, 2.5, 0.2, 10.0),
            Self::Haiku => ModelPrice::new(1.0, 1.25, 0.1, 5.0),
            Self::Fable => ModelPrice::new(10.0, 12.5, 1.0, 50.0),
        }
    }

    pub fn from_alias(alias: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.alias() == alias)
    }

    pub fn from_api_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.api_id() == id)
    }
}

/// A Codex model the catalog offers, in the order the Codex CLI's
/// `model/list` returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodexModel {
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
    pub const ALL: [Self; 7] = [
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
            Self::Gpt6Astra => "gpt-6-astra",
            Self::Gpt6Sol => "gpt-6-sol",
            Self::Gpt6Luna => "gpt-6-luna",
            Self::Gpt56Sol => "gpt-5.6-sol",
            Self::Gpt56Terra => "gpt-5.6-terra",
            Self::Gpt56Luna => "gpt-5.6-luna",
            Self::Gpt55 => "gpt-5.5",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Gpt6Astra => "GPT-6 Astra",
            Self::Gpt6Sol => "GPT-6 Sol",
            Self::Gpt6Luna => "GPT-6 Luna",
            Self::Gpt56Sol => "GPT-5.6 Sol",
            Self::Gpt56Terra => "GPT-5.6 Terra",
            Self::Gpt56Luna => "GPT-5.6 Luna",
            Self::Gpt55 => "GPT-5.5",
        }
    }

    /// The effort levels the model accepts, weakest first. Codex fails the
    /// whole turn on a level its model rejects rather than falling back.
    pub const fn effort_levels(self) -> &'static [&'static str] {
        match self {
            Self::Gpt6Astra | Self::Gpt6Sol | Self::Gpt56Sol | Self::Gpt56Terra => {
                CODEX_ULTRA_EFFORT_LEVELS
            }
            Self::Gpt6Luna | Self::Gpt56Luna => CODEX_MAX_EFFORT_LEVELS,
            Self::Gpt55 => CODEX_BASE_EFFORT_LEVELS,
        }
    }

    /// The GPT-6 and GPT-5.6 families at their short-context (<272K input)
    /// rates, which apply throughout.
    pub const fn price(self) -> ModelPrice {
        match self {
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

/// The Claude catalog alias a model id belongs to, or `None` for an id no
/// catalog entry names.
pub fn claude_model_alias(model_id: &str) -> Option<&'static str> {
    ClaudeModel::from_api_id(model_id).map(ClaudeModel::alias)
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

/// Sonnet before Sonnet 5, which costs half as much again.
const EARLIER_SONNET: ModelPrice = ModelPrice::new(3.0, 3.75, 0.3, 15.0);
/// Codex ids before the GPT-5.6 family. Cache-write is not a tier there (the
/// cache-creation count is always 0), so it rides the input rate.
const EARLIER_CODEX: ModelPrice = ModelPrice::new(1.25, 1.25, 0.125, 10.0);

/// Prices for a reported model id, or `None` for an unrecognised model.
///
/// A catalog model matches on its id anywhere in the reported one. An earlier
/// id no catalog model names is priced by its family: Opus, Haiku and Fable
/// generations share a price, Sonnet and Codex ones do not.
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
    if m.contains("codex") || m.contains("gpt-5") {
        return Some(EARLIER_CODEX);
    }
    if m.contains("sonnet") {
        return Some(EARLIER_SONNET);
    }
    ClaudeModel::ALL
        .into_iter()
        .find(|c| m.contains(c.alias()))
        .map(ClaudeModel::price)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_generation_is_not_priced_as_its_family() {
        assert_eq!(
            price_for("claude-sonnet-5"),
            Some(ClaudeModel::Sonnet.price())
        );
        assert_eq!(price_for("claude-sonnet-4-6"), Some(EARLIER_SONNET));
        assert_eq!(price_for("gpt-5.6-sol"), Some(CodexModel::Gpt56Sol.price()));
        assert_eq!(price_for("gpt-5-codex"), Some(EARLIER_CODEX));
    }

    #[test]
    fn an_earlier_generation_takes_its_family_price() {
        assert_eq!(
            price_for("claude-opus-4-8"),
            Some(ClaudeModel::Opus.price())
        );
        assert_eq!(
            price_for("claude-haiku-4-5"),
            Some(ClaudeModel::Haiku.price())
        );
        assert_eq!(price_for("gpt-6-nova"), None);
    }

    #[test]
    fn an_unlisted_codex_model_accepts_only_the_base_levels() {
        assert_eq!(codex_effort_levels("gpt-6-nova"), CODEX_BASE_EFFORT_LEVELS);
        assert!(codex_effort_levels("gpt-6-astra").contains(&"ultra"));
    }
}

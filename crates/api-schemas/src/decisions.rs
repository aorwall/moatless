//! Decision request/response types, independent of the decision provider.

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/v1/decisions/pick`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PickDecisionRequest {
    /// What to decide, e.g. "Which repository owns this issue?".
    pub question: String,
    /// The labels to pick from, in the caller's order.
    pub options: Vec<DecisionOption>,
    /// The text the decision is about. It is sent to the decision provider.
    pub state: String,
    /// Task the caller runs in, used only to attribute the decision in logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// One label a decision can pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DecisionOption {
    pub name: String,
    /// When this option applies. Sent to the decision provider as the option's
    /// criterion; without it the provider gets the name alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Response body for `POST /api/v1/decisions/pick`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PickDecisionResponse {
    /// The picked option's name.
    pub choice: String,
    /// The provider's confidence in `choice`, 0–1. `None` from a provider that
    /// reports no calibrated probability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub band: DecisionBand,
    /// Per-option probabilities, most likely first; empty when the provider
    /// reports none.
    #[serde(default)]
    pub probabilities: Vec<OptionProbability>,
    /// Decision provider that answered, e.g. `jev`.
    pub provider: String,
    /// Model version that answered, e.g. `jev-1.13.0`.
    pub model: String,
    pub usage: DecisionUsage,
}

/// What a caller should do with a decision, from the provider's own
/// confidence thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DecisionBand {
    /// Confident enough to act on.
    Act,
    /// Plausible; check it before acting.
    Confirm,
    /// Too uncertain; decide yourself.
    Escalate,
}

impl DecisionBand {
    /// The wire name, for plain-text output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Act => "act",
            Self::Confirm => "confirm",
            Self::Escalate => "escalate",
        }
    }
}

/// One option's probability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct OptionProbability {
    pub name: String,
    pub probability: f64,
}

/// Tokens the decision provider billed for one decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DecisionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

//! Decision errors and their HTTP mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use moatless_core::error::{causal_chain, error_response};

use crate::provider::ProviderError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DecisionError {
    #[error(
        "no decision provider is configured on this server; set DECISION_PROVIDER to enable decisions"
    )]
    Unconfigured,
    #[error("{0}")]
    Invalid(String),
    #[error("too many decisions in flight; retry shortly")]
    Busy,
    #[error("the decision provider picked an option that was not offered")]
    UnknownChoice,
    #[error("decision provider call failed")]
    Provider(#[from] ProviderError),
}

impl DecisionError {
    /// The `outcome` label on `moatless_decision_requests_total`.
    pub(crate) fn outcome(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Invalid(_) => "invalid",
            Self::Busy => "busy",
            Self::UnknownChoice => "unknown_choice",
            Self::Provider(e) => match e {
                ProviderError::Auth { .. } => "auth",
                ProviderError::Rejected => "rejected",
                ProviderError::Overloaded { .. } => "overloaded",
                ProviderError::Upstream { .. } => "upstream",
                ProviderError::Transport(_) => "transport",
                ProviderError::Decode(_) | ProviderError::Malformed(_) => "malformed",
            },
        }
    }
}

// A provider 401 must never answer 401: the CLI discards the caller's own
// session on any 401 (`crates/cli/src/invocation.rs`).
impl IntoResponse for DecisionError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Unconfigured | Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::Invalid(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Self::Provider(ProviderError::Rejected) => (
                StatusCode::BAD_REQUEST,
                "the decision provider rejected the request as invalid or too long".to_string(),
            ),
            Self::Provider(ProviderError::Overloaded { .. }) => {
                tracing::warn!(error = %causal_chain(&self), "Decision provider overloaded");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the decision provider is overloaded; retry shortly".to_string(),
                )
            }
            Self::UnknownChoice | Self::Provider(_) => {
                tracing::error!(error = %causal_chain(&self), "Decision provider failed");
                (
                    StatusCode::BAD_GATEWAY,
                    "the decision provider failed".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}

//! The decision providers the backend can call, and the gate in front of them.

use std::time::Duration;

use moatless_api_schemas::decisions::{DecisionBand, DecisionOption, DecisionUsage};
use moatless_core::config::DecisionProviderConfig;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;

use crate::jev::JevClient;

/// One validated multiple-choice question.
pub(crate) struct Question<'a> {
    pub(crate) text: &'a str,
    /// Trimmed, unique names in the caller's order.
    pub(crate) options: &'a [DecisionOption],
    pub(crate) state: &'a str,
}

/// A provider's answer before it is checked against the options and banded.
#[derive(Debug)]
pub(crate) struct ProviderPick {
    pub(crate) choice: String,
    pub(crate) confidence: Option<f64>,
    /// Unordered, and may name options that were not offered.
    pub(crate) probabilities: Vec<(String, f64)>,
    pub(crate) usage: DecisionUsage,
    pub(crate) attempts: u32,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProviderError {
    #[error("provider rejected the backend's credentials (HTTP {status})")]
    Auth { status: u16 },
    #[error("provider rejected the request as invalid or too long")]
    Rejected,
    #[error("provider overloaded (HTTP {status}) after {attempts} attempts")]
    Overloaded { status: u16, attempts: u32 },
    #[error("provider returned HTTP {status}")]
    Upstream { status: u16 },
    #[error("provider request failed")]
    Transport(#[source] reqwest::Error),
    #[error("provider answer did not decode")]
    Decode(#[source] serde_json::Error),
    #[error("provider answer malformed: {0}")]
    Malformed(&'static str),
}

/// One variant per [`DecisionProviderConfig`] variant.
///
/// Each provider owns its band thresholds and input budget, because
/// calibration and context size differ between models.
#[derive(Debug)]
pub(crate) enum DecisionProvider {
    Jev(JevClient),
}

impl DecisionProvider {
    pub(crate) fn from_config(config: &DecisionProviderConfig) -> Self {
        match config {
            DecisionProviderConfig::Jev(jev) => Self::Jev(JevClient::new(jev)),
        }
    }

    /// The name reported in responses, logs and metric labels.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Jev(_) => crate::jev::PROVIDER,
        }
    }

    pub(crate) fn model(&self) -> &str {
        match self {
            Self::Jev(client) => client.model(),
        }
    }

    /// Characters of question, state and options one call accepts.
    pub(crate) fn max_input_chars(&self) -> usize {
        match self {
            Self::Jev(_) => crate::jev::MAX_INPUT_CHARS,
        }
    }

    /// A missing confidence always escalates.
    pub(crate) fn band(&self, confidence: Option<f64>) -> DecisionBand {
        let Some(confidence) = confidence else {
            return DecisionBand::Escalate;
        };
        let (act, confirm) = match self {
            Self::Jev(_) => (crate::jev::ACT_AT, crate::jev::CONFIRM_AT),
        };
        if confidence >= act {
            DecisionBand::Act
        } else if confidence >= confirm {
            DecisionBand::Confirm
        } else {
            DecisionBand::Escalate
        }
    }

    /// Answers `question`, retrying as the provider's policy allows until
    /// `deadline`.
    pub(crate) async fn pick(
        &self,
        question: &Question<'_>,
        deadline: Instant,
    ) -> Result<ProviderPick, ProviderError> {
        match self {
            Self::Jev(client) => client.pick(question, deadline).await,
        }
    }
}

/// A provider behind a bound on concurrent upstream calls.
#[derive(Debug)]
pub(crate) struct Decider {
    pub(crate) provider: DecisionProvider,
    permits: Semaphore,
}

impl Decider {
    pub(crate) fn new(provider: DecisionProvider, max_in_flight: usize) -> Self {
        Self {
            provider,
            permits: Semaphore::new(max_in_flight),
        }
    }

    /// Returns `None` when no slot frees up within `wait`.
    pub(crate) async fn admit(&self, wait: Duration) -> Option<SemaphorePermit<'_>> {
        tokio::time::timeout(wait, self.permits.acquire())
            .await
            .ok()?
            .ok()
    }
}

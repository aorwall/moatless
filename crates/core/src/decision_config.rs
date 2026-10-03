//! The decision provider's settings, parsed from the values `Config` reads.

use std::fmt;

use crate::config::ConfigError;

/// Pinned so an upstream `jev-latest` bump never changes decisions silently.
pub const DEFAULT_JEV_MODEL: &str = "jev-1.13.0";

const DEFAULT_TYPESAFE_BASE_URL: &str = "https://api.typesafe.ai";

/// The decision provider behind `POST /api/v1/decisions/pick`; `None` (env
/// `DECISION_PROVIDER` unset or empty) disables the endpoint.
#[derive(Debug, Clone)]
pub struct DecisionConfig {
    pub provider: Option<DecisionProviderConfig>,
}

/// One variant per decision provider, each carrying its own credentials.
#[derive(Debug, Clone)]
pub enum DecisionProviderConfig {
    /// TypeSafe Jev, `DECISION_PROVIDER=jev`.
    Jev(JevConfig),
}

/// TypeSafe Jev settings. Implements a custom `Debug` to redact the key.
#[derive(Clone)]
pub struct JevConfig {
    /// Env `TYPESAFE_API_KEY`. Backend-only: no sandbox ever receives it.
    pub api_key: String,
    /// Env `TYPESAFE_API_BASE_URL` (default `https://api.typesafe.ai`).
    pub base_url: String,
    /// Env `DECISION_MODEL` (default [`DEFAULT_JEV_MODEL`]).
    pub model: String,
}

impl fmt::Debug for JevConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevConfig")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish()
    }
}

impl DecisionConfig {
    /// An empty value counts as unset: docker-compose passes `${VAR:-}` as `""`.
    pub(crate) fn parse(
        provider: Option<String>,
        api_key: Option<String>,
        base_url: Option<String>,
        model: Option<String>,
    ) -> Result<Self, ConfigError> {
        let set = |value: Option<String>| {
            value
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let Some(provider) = set(provider) else {
            return Ok(Self { provider: None });
        };
        match provider.to_ascii_lowercase().as_str() {
            "jev" => {
                let api_key = set(api_key).ok_or_else(|| {
                    ConfigError::Invalid("DECISION_PROVIDER=jev requires TYPESAFE_API_KEY".into())
                })?;
                let base_url = set(base_url).map_or_else(
                    || DEFAULT_TYPESAFE_BASE_URL.to_string(),
                    |url| url.trim_end_matches('/').to_string(),
                );
                let model = set(model).unwrap_or_else(|| DEFAULT_JEV_MODEL.to_string());
                Ok(Self {
                    provider: Some(DecisionProviderConfig::Jev(JevConfig {
                        api_key,
                        base_url,
                        model,
                    })),
                })
            }
            other => Err(ConfigError::Invalid(format!(
                "unknown DECISION_PROVIDER '{other}'; expected jev"
            ))),
        }
    }
}

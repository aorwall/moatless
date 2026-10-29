//! The model gateway's settings, parsed from the values `Config` reads.

use std::fmt;

/// The backend's model gateway, which a pod's agent reaches model providers
/// through with its task token as the key.
///
/// Implements a custom `Debug` to redact the hub key.
#[derive(Clone, Default)]
pub struct ModelGatewayConfig {
    /// `MODEL_GATEWAY_ENABLED`: whether pods are told to route through it.
    pub enabled: bool,
    /// `MODEL_GATEWAY_HUB_URL`, the deployment's `CLIProxyAPI` hub, without a
    /// trailing `/`.
    pub hub_url: Option<String>,
    /// `MODEL_GATEWAY_HUB_KEY`, an inference api-key the hub minted: not its
    /// management key.
    pub hub_key: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for ModelGatewayConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelGatewayConfig")
            .field("enabled", &self.enabled)
            .field("hub_url", &self.hub_url)
            .field("hub_key", &self.hub_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

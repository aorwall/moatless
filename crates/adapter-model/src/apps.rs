//! The deployment-wide Adapter app record, and the seam an Adapter reads it
//! through.
//!
//! An Adapter needs one thing from the stored app: the secrets and settings a
//! deployment saved under its `app_key`. What the Adapter makes of them — a bot
//! token, a signing secret, an OAuth client pair — is that Adapter's own shape,
//! so each one reads the fields it needs out of [`AdapterAppSecrets`]. The crate
//! that stores the rows names no provider.

use std::collections::HashMap;

use async_trait::async_trait;
use moatless_types::store::StoreError;
use serde_json::Value;

/// The app a deployment gets when it names no `app_key`.
pub const DEFAULT_APP_KEY: &str = "default";

/// The app key a connection's transport config names, or [`DEFAULT_APP_KEY`].
#[must_use]
pub fn app_key_from_transport_config(transport_config: &Value) -> &str {
    transport_config
        .get("app_key")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_APP_KEY)
}

/// One stored Adapter app, decrypted.
#[derive(Clone, Debug)]
pub struct AdapterAppSecrets {
    /// The key that selects this app among the ones stored for its provider.
    pub app_key: String,
    /// The decrypted secret values, by name.
    pub secrets: HashMap<String, String>,
    /// The non-secret half of the app's configuration.
    pub config: Value,
}

impl AdapterAppSecrets {
    /// A decrypted secret by name.
    #[must_use]
    pub fn secret(&self, name: &str) -> Option<String> {
        self.secrets.get(name).cloned()
    }

    /// A string setting from the non-secret half.
    #[must_use]
    pub fn setting(&self, name: &str) -> Option<String> {
        self.config
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

/// The stored Adapter apps, narrowed to the one read an Adapter makes.
#[async_trait]
pub trait AdapterAppReader: Send + Sync {
    /// The enabled app of this kind stored under `app_key`, or `None` when the
    /// deployment stored none.
    async fn app(
        &self,
        adapter_kind: &str,
        app_key: &str,
    ) -> Result<Option<AdapterAppSecrets>, StoreError>;
}

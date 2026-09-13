//! Deployment-wide provider app credentials, described as data.
//!
//! A provider is registered by publishing a [`ProviderAppSpec`] rather than by
//! adding a request type, so a client renders a form and a CLI takes fields for
//! a provider it has never heard of.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One field a provider's app takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderAppField {
    /// Its name on the wire, and the name `moat app set --field <name>=` takes.
    pub name: String,
    pub label: String,
    pub description: String,
    /// Whether the stored value is sealed and never returned by a read path.
    pub secret: bool,
    pub required: bool,
}

/// What one provider needs before its app credentials can be stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderAppSpec {
    /// The `{provider}` path segment: `forgejo`, `slack`, `telegram`, `linear`.
    pub provider: String,
    pub display_name: String,
    /// What the `{key}` path segment names here — a Forgejo host, an adapter's
    /// app key.
    pub key_label: String,
    pub key_description: String,
    /// The URL an admin must register at the provider itself. `None` where the
    /// provider needs none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    pub fields: Vec<ProviderAppField>,
}

impl ProviderAppSpec {
    pub fn field(&self, name: &str) -> Option<&ProviderAppField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// One registered app, carrying no secret value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderAppSummary {
    pub provider: String,
    pub key: String,
    pub display_name: String,
    /// The non-secret fields, as stored.
    pub config: BTreeMap<String, String>,
    /// A digest per stored secret, enough to tell two values apart and
    /// revealing neither. Empty where the store keeps no digest.
    pub secret_fingerprints: BTreeMap<String, String>,
    pub updated_at: String,
}

/// `GET /api/v1/admin/settings/apps` — every spec, and everything registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProviderAppsResponse {
    pub specs: Vec<ProviderAppSpec>,
    pub apps: Vec<ProviderAppSummary>,
}

/// `PUT /api/v1/admin/settings/apps/{provider}/{key}`.
///
/// Every value is a field named by the provider's spec; an unknown name is
/// rejected rather than dropped, so a mistyped secret cannot register an app
/// that authenticates against nothing.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderAppRequest {
    pub fields: BTreeMap<String, String>,
}

/// Redacts every value, not only the ones a spec marks secret: this type is
/// deserialized before any spec has been consulted.
impl std::fmt::Debug for SaveProviderAppRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveProviderAppRequest")
            .field(
                "fields",
                &self
                    .fields
                    .keys()
                    .map(|name| (name, "[REDACTED]"))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redacted_debug_prints_no_secret() {
        let request = SaveProviderAppRequest {
            fields: BTreeMap::from([
                ("clientId".to_string(), "Iv1.public".to_string()),
                ("clientSecret".to_string(), "gto_supersecret".to_string()),
            ]),
        };

        let printed = format!("{request:?}");
        assert!(!printed.contains("gto_supersecret"), "{printed}");
        // A public value is redacted too, and the names still print.
        assert!(!printed.contains("Iv1.public"), "{printed}");
        assert!(printed.contains("clientSecret"), "{printed}");
    }
}

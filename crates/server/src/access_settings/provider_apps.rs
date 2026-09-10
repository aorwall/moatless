//! One registration surface over every provider that holds deployment-wide app
//! credentials.
//!
//! A provider is an entry in [`SPECS`] plus an arm of each `match` below. The
//! store behind it stays whichever one already holds its rows: Forgejo keeps
//! `forgejo_instances`, the adapters keep `adapter_apps`, and nothing here
//! migrates either.

use std::collections::BTreeMap;

use moatless_api_schemas::provider_apps::{ProviderAppField, ProviderAppSpec, ProviderAppSummary};
use moatless_core::git_provider::GitProviderError;
use moatless_identity::adapter_apps::administration::{AdapterAppSave, require_admin};
use moatless_identity::forgejo::{ForgejoOAuth, ForgejoOAuthApp, callback_url, normalize_host};
use moatless_types::access_control::ScopeContext;

use crate::access_settings::error::{AccessSettingsError, auth_error};
use crate::state::AppState;

pub const FORGEJO: &str = "forgejo";
pub const SLACK: &str = "slack";
pub const TELEGRAM: &str = "telegram";
pub const LINEAR: &str = "linear";

/// A [`ProviderAppSpec`] before the deployment's own URLs are filled into it.
struct SpecEntry {
    provider: &'static str,
    display_name: &'static str,
    key_label: &'static str,
    key_description: &'static str,
    fields: &'static [FieldEntry],
}

struct FieldEntry {
    name: &'static str,
    label: &'static str,
    description: &'static str,
    secret: bool,
    required: bool,
}

/// Every provider this endpoint registers.
///
/// A GitHub App is absent deliberately: registering one takes a PEM signing key
/// proved against GitHub before it is stored, which is not a flat string field.
/// `moat github-app` and `/api/v1/admin/settings/github/apps` own it.
const SPECS: &[SpecEntry] = &[
    SpecEntry {
        provider: FORGEJO,
        display_name: "Forgejo",
        key_label: "host",
        key_description: "The instance's hostname, as it appears in a repository's clone URL.",
        fields: &[
            FieldEntry {
                name: "clientId",
                label: "Client ID",
                description: "The OAuth2 application's client ID.",
                secret: false,
                required: true,
            },
            FieldEntry {
                name: "clientSecret",
                label: "Client secret",
                description: "Shown once by Forgejo when the application is created.",
                secret: true,
                required: true,
            },
        ],
    },
    SpecEntry {
        provider: SLACK,
        display_name: "Slack",
        key_label: "app key",
        key_description: "Only 'default' is supported.",
        fields: &[
            FieldEntry {
                name: "botToken",
                label: "Bot token",
                description: "The 'xoxb-' token from OAuth & Permissions.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "signingSecret",
                label: "Signing secret",
                description: "Verifies that an inbound request came from Slack.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "botUserId",
                label: "Bot user ID",
                description: "The bot's own 'U...' id, so its own messages are ignored.",
                secret: false,
                required: false,
            },
        ],
    },
    SpecEntry {
        provider: TELEGRAM,
        display_name: "Telegram",
        key_label: "app key",
        key_description: "Only 'default' is supported.",
        fields: &[
            FieldEntry {
                name: "botToken",
                label: "Bot token",
                description: "The token BotFather issued.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "mode",
                label: "Mode",
                description: "'webhook' or 'polling'. Defaults to 'webhook'.",
                secret: false,
                required: false,
            },
        ],
    },
    SpecEntry {
        provider: LINEAR,
        display_name: "Linear",
        key_label: "app key",
        key_description: "Only 'default' is supported.",
        fields: &[
            FieldEntry {
                name: "webhookSecret",
                label: "Webhook secret",
                description: "Verifies that an inbound webhook came from Linear.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthClientId",
                label: "OAuth client ID",
                description: "From the Linear OAuth application.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthClientSecret",
                label: "OAuth client secret",
                description: "From the Linear OAuth application.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthCallbackUrl",
                label: "OAuth callback URL",
                description: "The redirect URI registered on the Linear application.",
                secret: false,
                required: false,
            },
            FieldEntry {
                name: "apiBaseUrl",
                label: "API base URL",
                description: "Overrides Linear's default GraphQL endpoint.",
                secret: false,
                required: false,
            },
        ],
    },
];

/// Every provider spec, with this deployment's own callback URLs filled in.
pub fn specs(frontend_url: &str) -> Vec<ProviderAppSpec> {
    SPECS
        .iter()
        .map(|entry| ProviderAppSpec {
            provider: entry.provider.to_string(),
            display_name: entry.display_name.to_string(),
            key_label: entry.key_label.to_string(),
            key_description: entry.key_description.to_string(),
            callback_url: (entry.provider == FORGEJO).then(|| callback_url(frontend_url)),
            fields: entry
                .fields
                .iter()
                .map(|field| ProviderAppField {
                    name: field.name.to_string(),
                    label: field.label.to_string(),
                    description: field.description.to_string(),
                    secret: field.secret,
                    required: field.required,
                })
                .collect(),
        })
        .collect()
}

fn is_registered(provider: &str) -> bool {
    SPECS.iter().any(|entry| entry.provider == provider)
}

fn spec(provider: &str) -> Result<ProviderAppSpec, AccessSettingsError> {
    specs("")
        .into_iter()
        .find(|spec| spec.provider == provider)
        .ok_or_else(|| {
            let known = SPECS
                .iter()
                .map(|entry| entry.provider)
                .collect::<Vec<_>>()
                .join(", ");
            AccessSettingsError::NotFound(format!(
                "'{provider}' is not a provider this deployment registers apps for. Known: {known}"
            ))
        })
}

/// `fields`, checked against `spec` and trimmed.
///
/// An unrecognised name is rejected rather than dropped: a mistyped secret name
/// would otherwise register an app that authenticates against nothing. A field
/// left empty is treated as absent, so clearing one and omitting one agree.
///
/// Every value is trimmed, including a secret. No field here is multi-line, and
/// a secret pasted with a trailing newline is the failure that costs an hour.
fn validate(
    spec: &ProviderAppSpec,
    fields: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AccessSettingsError> {
    let mut checked = BTreeMap::new();
    for (name, value) in fields {
        if spec.field(&name).is_none() {
            let known = spec
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(AccessSettingsError::BadRequest(format!(
                "'{name}' is not a field of the {} app. Fields: {known}",
                spec.display_name
            )));
        }
        let value = value.trim().to_string();
        if !value.is_empty() {
            checked.insert(name, value);
        }
    }
    for field in &spec.fields {
        if field.required && !checked.contains_key(&field.name) {
            return Err(AccessSettingsError::BadRequest(format!(
                "The {} app needs '{}' ({})",
                spec.display_name, field.name, field.label
            )));
        }
    }
    Ok(checked)
}

/// Every app registered under any provider in [`SPECS`].
pub async fn list(
    state: &AppState,
    ctx: &ScopeContext,
) -> Result<Vec<ProviderAppSummary>, AccessSettingsError> {
    require_admin(ctx, "read")?;
    let mut apps = Vec::new();
    for instance in state.forgejo_instances.list().await.map_err(auth_error)? {
        apps.push(ProviderAppSummary {
            provider: FORGEJO.to_string(),
            key: instance.host,
            display_name: "Forgejo".to_string(),
            config: BTreeMap::from([("clientId".to_string(), instance.client_id)]),
            // `forgejo_instances` stores no digest of the client secret.
            secret_fingerprints: BTreeMap::new(),
            updated_at: instance.updated_at.to_rfc3339(),
        });
    }
    for app in state.app_administration.list_adapter_apps(ctx).await? {
        if !is_registered(&app.adapter_kind) {
            continue;
        }
        apps.push(ProviderAppSummary {
            provider: app.adapter_kind,
            key: app.app_key,
            display_name: app.display_name,
            config: config_map(&app.config_json),
            secret_fingerprints: app.secret_fingerprints.into_iter().collect(),
            updated_at: app.updated_at.to_rfc3339(),
        });
    }
    Ok(apps)
}

/// Store `fields` as `provider`'s app for `key`, replacing any already there.
///
/// Forgejo is checked against its instance first, so a pair it rejects is never
/// written. The adapters have no equivalent check that costs one request.
pub async fn save(
    state: &AppState,
    ctx: &ScopeContext,
    provider: &str,
    key: &str,
    fields: BTreeMap<String, String>,
) -> Result<ProviderAppSummary, AccessSettingsError> {
    require_admin(ctx, "save")?;
    let spec = spec(provider)?;
    let fields = validate(&spec, fields)?;

    if provider == FORGEJO {
        return save_forgejo(state, ctx, key, &fields).await;
    }
    let saved = state
        .app_administration
        .save_adapter_app(provider, key, adapter_save(provider, &fields)?, ctx)
        .await?;
    Ok(ProviderAppSummary {
        provider: saved.adapter_kind,
        key: saved.app_key,
        display_name: saved.display_name,
        config: config_map(&saved.config_json),
        secret_fingerprints: saved.secret_fingerprints.into_iter().collect(),
        updated_at: saved.updated_at.to_rfc3339(),
    })
}

/// Remove `provider`'s app for `key`. Returns whether one was registered.
pub async fn remove(
    state: &AppState,
    ctx: &ScopeContext,
    provider: &str,
    key: &str,
) -> Result<bool, AccessSettingsError> {
    require_admin(ctx, "delete")?;
    let spec = spec(provider)?;
    if spec.provider == FORGEJO {
        return state
            .forgejo_instances
            .remove(key)
            .await
            .map_err(auth_error);
    }
    // `delete_adapter_app` reports no row count, so the caller is told whether
    // anything was registered by reading first.
    let registered = state
        .app_administration
        .list_adapter_apps(ctx)
        .await?
        .into_iter()
        .any(|app| app.adapter_kind == provider && app.app_key == key);
    if !registered {
        return Ok(false);
    }
    state
        .app_administration
        .delete_adapter_app(ctx, provider, key)
        .await?;
    Ok(true)
}

async fn save_forgejo(
    state: &AppState,
    ctx: &ScopeContext,
    key: &str,
    fields: &BTreeMap<String, String>,
) -> Result<ProviderAppSummary, AccessSettingsError> {
    let host = normalize_host(key);
    if host.is_empty() {
        return Err(AccessSettingsError::BadRequest(
            "A Forgejo app is registered against a hostname".to_string(),
        ));
    }
    let app = ForgejoOAuthApp {
        host: host.clone(),
        client_id: required(fields, "clientId")?,
        client_secret: required(fields, "clientSecret")?,
    };
    let redirect_uri = callback_url(&state.config.auth.frontend_url);
    ForgejoOAuth::default()
        .verify_client_credentials(&app, &redirect_uri)
        .await
        .map_err(|error| forgejo_rejection(error, &host, &redirect_uri))?;

    let registered = state
        .forgejo_instances
        .register(&host, &app.client_id, &app.client_secret, &ctx.user_id)
        .await
        .map_err(auth_error)?;
    Ok(ProviderAppSummary {
        provider: FORGEJO.to_string(),
        key: registered.host,
        display_name: "Forgejo".to_string(),
        config: BTreeMap::from([("clientId".to_string(), registered.client_id)]),
        secret_fingerprints: BTreeMap::new(),
        updated_at: registered.updated_at.to_rfc3339(),
    })
}

/// A pair Forgejo itself refused is the admin's to fix, so it answers 400 and
/// says what to look at. Anything else is the instance being unreachable.
fn forgejo_rejection(
    error: GitProviderError,
    host: &str,
    redirect_uri: &str,
) -> AccessSettingsError {
    if let GitProviderError::OAuthRejected { description, .. } = &error {
        return AccessSettingsError::BadRequest(format!(
            "{host} rejected the application: {description}. Check the client ID and secret, and \
             that the application is a confidential client listing {redirect_uri} as its redirect \
             URI."
        ));
    }
    AccessSettingsError::GitProvider(error)
}

fn adapter_save(
    provider: &str,
    fields: &BTreeMap<String, String>,
) -> Result<AdapterAppSave, AccessSettingsError> {
    match provider {
        SLACK => Ok(AdapterAppSave::Slack {
            bot_token: required(fields, "botToken")?,
            signing_secret: required(fields, "signingSecret")?,
            bot_user_id: fields.get("botUserId").cloned(),
        }),
        TELEGRAM => Ok(AdapterAppSave::Telegram {
            bot_token: required(fields, "botToken")?,
            mode: fields.get("mode").cloned(),
        }),
        LINEAR => Ok(AdapterAppSave::Linear {
            webhook_secret: fields.get("webhookSecret").cloned(),
            oauth_client_id: fields.get("oauthClientId").cloned(),
            oauth_client_secret: fields.get("oauthClientSecret").cloned(),
            oauth_callback_url: fields.get("oauthCallbackUrl").cloned(),
            api_base_url: fields.get("apiBaseUrl").cloned(),
        }),
        other => Err(AccessSettingsError::NotFound(format!(
            "'{other}' registers no adapter app"
        ))),
    }
}

/// A field [`validate`] has already proved present.
fn required(fields: &BTreeMap<String, String>, name: &str) -> Result<String, AccessSettingsError> {
    fields
        .get(name)
        .cloned()
        .ok_or_else(|| AccessSettingsError::BadRequest(format!("The app needs '{name}'")))
}

/// `config_json` flattened to the strings a summary shows. A `null` is dropped,
/// so an unset optional reads the same as one never written.
fn config_map(config: &serde_json::Value) -> BTreeMap<String, String> {
    let Some(object) = config.as_object() else {
        return BTreeMap::new();
    };
    object
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| {
            let value = match value.as_str() {
                Some(text) => text.to_string(),
                None => value.to_string(),
            };
            (name.clone(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forgejo_spec() -> ProviderAppSpec {
        spec(FORGEJO).expect("forgejo is a registered provider")
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn an_unknown_field_is_rejected() {
        let error = validate(
            &forgejo_spec(),
            fields(&[("clientId", "id"), ("clientSecrets", "shh")]),
        )
        .expect_err("a misspelt field is not stored silently");

        let message = error.to_string();
        assert!(message.contains("clientSecrets"), "{message}");
        // The reply names what it would have accepted.
        assert!(message.contains("clientSecret"), "{message}");
    }

    #[test]
    fn a_missing_required_field_is_rejected() {
        let error = validate(&forgejo_spec(), fields(&[("clientId", "id")]))
            .expect_err("a client secret is required");
        assert!(error.to_string().contains("clientSecret"), "{error}");

        // Present but empty is the same thing, so clearing a field says so.
        let error = validate(
            &forgejo_spec(),
            fields(&[("clientId", "id"), ("clientSecret", "  ")]),
        )
        .expect_err("an empty client secret is no client secret");
        assert!(error.to_string().contains("clientSecret"), "{error}");
    }

    #[test]
    fn a_value_is_stored_without_the_whitespace_it_was_pasted_with() {
        let checked = validate(
            &forgejo_spec(),
            fields(&[("clientId", " id\n"), ("clientSecret", "shh\n")]),
        )
        .expect("a pasted pair registers");
        assert_eq!(checked.get("clientId"), Some(&"id".to_string()));
        assert_eq!(checked.get("clientSecret"), Some(&"shh".to_string()));
    }

    #[test]
    fn a_forgejo_key_is_normalized_like_a_remote_url() {
        assert_eq!(normalize_host("https://Git.Acme.dev/"), "git.acme.dev");
        assert!(normalize_host("https://").is_empty());
    }

    #[test]
    fn every_spec_field_maps_to_its_adapter_app_save_variant() {
        for provider in [SLACK, TELEGRAM, LINEAR] {
            let spec = spec(provider).expect("a registered provider");
            let given = spec
                .fields
                .iter()
                .map(|field| (field.name.clone(), format!("value-of-{}", field.name)))
                .collect::<BTreeMap<_, _>>();

            let checked = validate(&spec, given.clone()).expect("every field is its own");
            let save = adapter_save(provider, &checked).expect("a save input");

            let printed = format!("{save:?}");
            for value in given.values() {
                assert!(printed.contains(value), "{provider} dropped {value}");
            }
        }
    }

    #[test]
    fn only_forgejo_publishes_a_callback_url() {
        let specs = specs("https://app.example/");
        let forgejo = specs
            .iter()
            .find(|spec| spec.provider == FORGEJO)
            .expect("forgejo is a registered provider");
        assert_eq!(
            forgejo.callback_url.as_deref(),
            Some("https://app.example/api/v1/auth/forgejo/callback")
        );
        // Linear's redirect URI is a field an admin supplies, not one derived.
        for spec in specs.iter().filter(|spec| spec.provider != FORGEJO) {
            assert_eq!(spec.callback_url, None, "{}", spec.provider);
        }
    }

    #[test]
    fn a_config_value_that_was_never_set_is_absent_rather_than_null() {
        let config = serde_json::json!({ "mode": "webhook", "bot_user_id": null });
        assert_eq!(
            config_map(&config),
            BTreeMap::from([("mode".to_string(), "webhook".to_string())])
        );
    }

    #[test]
    fn a_rejected_forgejo_pair_is_the_callers_to_fix() {
        let error = forgejo_rejection(
            GitProviderError::OAuthRejected {
                provider: "Forgejo",
                code: "unauthorized_client".to_string(),
                description: "invalid client secret".to_string(),
            },
            "git.acme.dev",
            "https://app.example/api/v1/auth/forgejo/callback",
        );
        assert!(matches!(error, AccessSettingsError::BadRequest(_)));
        let message = error.to_string();
        assert!(message.contains("invalid client secret"), "{message}");
        assert!(message.contains("redirect URI"), "{message}");
    }
}

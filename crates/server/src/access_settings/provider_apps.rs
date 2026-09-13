//! One registration surface over every provider that holds deployment-wide app
//! credentials.
//!
//! A provider is an entry in [`SPECS`] plus an arm of each `match` below. The
//! store behind it stays whichever one already holds its rows: Forgejo keeps
//! `forgejo_instances`, the adapters keep `adapter_apps`, and nothing here
//! migrates either.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use moatless_api_schemas::provider_apps::{ProviderAppField, ProviderAppSpec, ProviderAppSummary};
use moatless_core::git_provider::GitProviderError;
use moatless_identity::adapter_apps::administration::{
    AdapterAppSave, AppAdministration, require_admin,
};
use moatless_identity::forgejo::{
    ForgejoInstances, ForgejoOAuth, ForgejoOAuthApp, callback_url, normalize_host,
};
use moatless_types::access_control::ScopeContext;

use crate::access_settings::error::{AccessSettingsError, auth_error};

/// The two stores a provider app lands in, and the origin its Forgejo redirect
/// URI ([`callback_url`]) is derived from.
///
/// Extracted as `State<ProviderApps>` via the `FromRef` impl in `crate::state`.
pub struct ProviderApps {
    pub forgejo_instances: Arc<ForgejoInstances>,
    pub app_administration: Arc<AppAdministration>,
    pub frontend_url: String,
}

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
    /// The name the provider's own store writes this field under.
    ///
    /// [`rekey`] maps it back to [`FieldEntry::name`] on every read, so a name
    /// [`ProviderAppSummary`] returns is one [`save`] accepts back.
    stored_as: &'static str,
    label: &'static str,
    description: &'static str,
    secret: bool,
    required: bool,
}

/// Forgejo's own entry, named so the read paths that build a Forgejo summary
/// without an adapter row still take its display name and its field names from
/// [`SPECS`].
const FORGEJO_SPEC: SpecEntry = SpecEntry {
    provider: FORGEJO,
    display_name: "Forgejo",
    key_label: "host",
    key_description: "The instance's hostname, as it appears in a repository's clone URL.",
    fields: &[
        FieldEntry {
            name: "clientId",
            stored_as: "client_id",
            label: "Client ID",
            description: "The OAuth2 application's client ID.",
            secret: false,
            required: true,
        },
        FieldEntry {
            name: "clientSecret",
            stored_as: "client_secret",
            label: "Client secret",
            description: "Shown once by Forgejo when the application is created.",
            secret: true,
            required: true,
        },
    ],
};

/// Every provider this endpoint registers.
///
/// A GitHub App is absent deliberately: it needs a PEM signing key, not a flat
/// string field; `moat github-app` and the admin GitHub-apps endpoint own it.
const SPECS: &[SpecEntry] = &[
    FORGEJO_SPEC,
    SpecEntry {
        provider: SLACK,
        display_name: "Slack",
        key_label: "app key",
        key_description: "Only 'default' is supported.",
        fields: &[
            FieldEntry {
                name: "botToken",
                stored_as: "bot_token",
                label: "Bot token",
                description: "The 'xoxb-' token from OAuth & Permissions.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "signingSecret",
                stored_as: "signing_secret",
                label: "Signing secret",
                description: "Verifies that an inbound request came from Slack.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "botUserId",
                stored_as: "bot_user_id",
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
                stored_as: "bot_token",
                label: "Bot token",
                description: "The token BotFather issued.",
                secret: true,
                required: true,
            },
            FieldEntry {
                name: "mode",
                stored_as: "mode",
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
                stored_as: "webhook_secret",
                label: "Webhook secret",
                description: "Verifies that an inbound webhook came from Linear.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthClientId",
                stored_as: "oauth_client_id",
                label: "OAuth client ID",
                description: "From the Linear OAuth application.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthClientSecret",
                stored_as: "oauth_client_secret",
                label: "OAuth client secret",
                description: "From the Linear OAuth application.",
                secret: true,
                required: false,
            },
            FieldEntry {
                name: "oauthCallbackUrl",
                stored_as: "oauth_callback_url",
                label: "OAuth callback URL",
                description: "The redirect URI registered on the Linear application.",
                secret: false,
                required: false,
            },
            FieldEntry {
                name: "apiBaseUrl",
                stored_as: "api_base_url",
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
        .map(|entry| spec_of(entry, frontend_url))
        .collect()
}

fn spec_of(entry: &SpecEntry, frontend_url: &str) -> ProviderAppSpec {
    ProviderAppSpec {
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
    }
}

fn entry(provider: &str) -> Option<&'static SpecEntry> {
    SPECS.iter().find(|entry| entry.provider == provider)
}

fn registered(provider: &str) -> Result<&'static SpecEntry, AccessSettingsError> {
    entry(provider).ok_or_else(|| {
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

/// `stored` keyed by [`FieldEntry::name`] rather than
/// [`FieldEntry::stored_as`].
///
/// A key no field claims is returned unchanged, so an unpublished stored name
/// still reaches the reader.
fn rekey(entry: &SpecEntry, stored: BTreeMap<String, String>) -> BTreeMap<String, String> {
    stored
        .into_iter()
        .map(
            |(name, value)| match entry.fields.iter().find(|field| field.stored_as == name) {
                Some(field) => (field.name.to_string(), value),
                None => (name, value),
            },
        )
        .collect()
}

/// `fields`, checked against `spec` and trimmed.
///
/// An unrecognised name is rejected, not dropped, so a mistyped secret name
/// never registers an app that authenticates against nothing; every value,
/// including secrets, is trimmed, and an empty one is treated as absent.
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
    apps_state: &ProviderApps,
    ctx: &ScopeContext,
) -> Result<Vec<ProviderAppSummary>, AccessSettingsError> {
    require_admin(ctx, "read")?;
    let mut apps = Vec::new();
    for instance in apps_state
        .forgejo_instances
        .list()
        .await
        .map_err(auth_error)?
    {
        apps.push(forgejo_summary(
            instance.host,
            instance.client_id,
            instance.updated_at,
        ));
    }
    for app in apps_state.app_administration.list_adapter_apps(ctx).await? {
        let Some(entry) = entry(&app.adapter_kind) else {
            continue;
        };
        apps.push(ProviderAppSummary {
            provider: app.adapter_kind,
            key: app.app_key,
            display_name: app.display_name,
            config: rekey(entry, config_map(&app.config_json)),
            secret_fingerprints: rekey(entry, app.secret_fingerprints.into_iter().collect()),
            updated_at: app.updated_at.to_rfc3339(),
        });
    }
    Ok(apps)
}

/// Store `fields` as `provider`'s app for `key`.
///
/// **A field the request omits is cleared, not left as it was**: each save
/// rewrites the row whole, so `moat app set` needs `--clear-omitted` to drop a
/// stored secret on purpose.
pub async fn save(
    apps_state: &ProviderApps,
    ctx: &ScopeContext,
    provider: &str,
    key: &str,
    fields: BTreeMap<String, String>,
) -> Result<ProviderAppSummary, AccessSettingsError> {
    require_admin(ctx, "save")?;
    let entry = registered(provider)?;
    let spec = spec_of(entry, &apps_state.frontend_url);
    let fields = validate(&spec, fields)?;

    if entry.provider == FORGEJO {
        return save_forgejo(apps_state, ctx, key, &fields).await;
    }
    let saved = apps_state
        .app_administration
        .save_adapter_app(provider, key, adapter_save(provider, &fields)?, ctx)
        .await?;
    Ok(ProviderAppSummary {
        provider: saved.adapter_kind,
        key: saved.app_key,
        display_name: saved.display_name,
        config: rekey(entry, config_map(&saved.config_json)),
        secret_fingerprints: rekey(entry, saved.secret_fingerprints.into_iter().collect()),
        updated_at: saved.updated_at.to_rfc3339(),
    })
}

/// Remove `provider`'s app for `key`. Returns whether one was registered.
pub async fn remove(
    apps_state: &ProviderApps,
    ctx: &ScopeContext,
    provider: &str,
    key: &str,
) -> Result<bool, AccessSettingsError> {
    require_admin(ctx, "delete")?;
    let entry = registered(provider)?;
    if entry.provider == FORGEJO {
        return apps_state
            .forgejo_instances
            .remove(key)
            .await
            .map_err(auth_error);
    }
    // `delete_adapter_app` reports no row count, so the caller is told whether
    // anything was registered by reading first.
    let registered = apps_state
        .app_administration
        .list_adapter_apps(ctx)
        .await?
        .into_iter()
        .any(|app| app.adapter_kind == provider && app.app_key == key);
    if !registered {
        return Ok(false);
    }
    apps_state
        .app_administration
        .delete_adapter_app(ctx, provider, key)
        .await?;
    Ok(true)
}

async fn save_forgejo(
    apps_state: &ProviderApps,
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
    let redirect_uri = callback_url(&apps_state.frontend_url);
    ForgejoOAuth::default()
        .verify_client_credentials(&app, &redirect_uri)
        .await
        .map_err(|error| forgejo_rejection(error, &host, &redirect_uri))?;

    let instance = apps_state
        .forgejo_instances
        .register(&host, &app.client_id, &app.client_secret, &ctx.user_id)
        .await
        .map_err(auth_error)?;
    Ok(forgejo_summary(
        instance.host,
        instance.client_id,
        instance.updated_at,
    ))
}

/// A Forgejo registration as a summary. `forgejo_instances` returns the column
/// names, so [`rekey`] carries them to the names [`FORGEJO_SPEC`] publishes.
fn forgejo_summary(
    host: String,
    client_id: String,
    updated_at: DateTime<Utc>,
) -> ProviderAppSummary {
    ProviderAppSummary {
        provider: FORGEJO.to_string(),
        key: host,
        display_name: FORGEJO_SPEC.display_name.to_string(),
        config: rekey(
            &FORGEJO_SPEC,
            BTreeMap::from([("client_id".to_string(), client_id)]),
        ),
        // `forgejo_instances` stores no digest of the client secret.
        secret_fingerprints: BTreeMap::new(),
        updated_at: updated_at.to_rfc3339(),
    }
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
    use moatless_identity::adapter_apps::administration::adapter_app_input;

    use super::*;

    fn spec_for(provider: &str) -> ProviderAppSpec {
        let entry = registered(provider).expect("a registered provider");
        spec_of(entry, "https://app.example")
    }

    fn forgejo_spec() -> ProviderAppSpec {
        spec_for(FORGEJO)
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
            let spec = spec_for(provider);
            let given = spec
                .fields
                .iter()
                .map(|field| {
                    // `adapter_app_input` checks Telegram's mode against its two
                    // legal values, so this one field takes a real one.
                    let value = match field.name.as_str() {
                        "mode" => "webhook".to_string(),
                        name => format!("value-of-{name}"),
                    };
                    (field.name.clone(), value)
                })
                .collect::<BTreeMap<_, _>>();

            let checked = validate(&spec, given.clone()).expect("every field is its own");
            let save = adapter_save(provider, &checked).expect("a save input");

            let printed = format!("{save:?}");
            for value in given.values() {
                assert!(printed.contains(value), "{provider} dropped {value}");
            }

            // The names the store writes, read off the input it is given rather
            // than off `AdapterAppSave`'s field idents. A key renamed in
            // `adapter_app_input` alone stops `rekey` translating, and this is
            // what catches it.
            let input = adapter_app_input(provider, "default", save).expect("a store input");
            for field in registered(provider).expect("a registered provider").fields {
                let stored = input
                    .config_json
                    .get(field.stored_as)
                    .is_some_and(|value| !value.is_null())
                    || input
                        .secrets
                        .iter()
                        .any(|secret| secret.name == field.stored_as);
                assert!(
                    stored,
                    "{provider} stores no '{}' for '{}'",
                    field.stored_as, field.name
                );
            }
        }
    }

    #[test]
    fn a_stored_value_reads_back_under_the_name_the_spec_publishes() {
        // A summary a client cannot feed back to `save` is a form it cannot
        // pre-fill, so every stored name maps onto a field the spec names.
        for entry in SPECS {
            let stored = entry
                .fields
                .iter()
                .map(|field| (field.stored_as.to_string(), "kept".to_string()))
                .collect();
            let read = rekey(entry, stored);
            let spec = spec_for(entry.provider);
            for name in read.keys() {
                assert!(
                    spec.field(name).is_some(),
                    "{} returns '{name}', which is not one of its fields",
                    entry.provider
                );
            }
            assert_eq!(read.len(), entry.fields.len(), "{}", entry.provider);
        }

        let summary = forgejo_summary(
            "git.acme.dev".to_string(),
            "Iv1.abc".to_string(),
            Utc::now(),
        );
        assert_eq!(summary.config.get("clientId"), Some(&"Iv1.abc".to_string()));
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

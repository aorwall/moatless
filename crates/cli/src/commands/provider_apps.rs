//! Provider app administration — `moat app list|spec|set|remove`.
//!
//! The backend publishes what each provider needs, so `--field name=value`
//! takes whatever the deployment's specs name and this command needs no edit
//! when a provider is added.
//!
//! A secret is never taken inline by default — it lands in shell history and
//! the process table — so it's prompted for, or read from a file with `@path`.

use std::collections::BTreeMap;
use std::io::Read;

use clap::Subcommand;
use moatless_api_schemas::provider_apps::{
    ProviderAppSpec, ProviderAppSummary, ProviderAppsResponse,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

const PATH: &str = "/api/v1/admin/settings/apps";

#[derive(Subcommand)]
pub enum ProviderAppAction {
    /// List every provider this deployment registers apps for, and what is
    /// registered under each.
    List {
        /// Show one provider only.
        #[arg(long)]
        provider: Option<String>,
    },

    /// Show the fields one provider's app takes.
    Spec {
        /// A provider named by `moat app list`, e.g. `forgejo`.
        provider: String,
    },

    /// Register or replace an app.
    ///
    /// A save replaces every field, so an omitted one is cleared — except a
    /// secret, which can't be read back: an omitted required secret is prompted
    /// for, and an omitted optional one is left alone unless `--clear-omitted`
    /// says to drop it. The provider confirms credentials where it can; nothing
    /// is stored when it refuses them.
    Set {
        /// A provider named by `moat app list`, e.g. `forgejo`.
        provider: String,

        /// What the app is keyed by: a Forgejo host, or an adapter's app key.
        key: String,

        /// `name=value`, repeated once per field. `name=@path` reads the value
        /// from a file. `name=-` reads it from standard input, which serves one
        /// field per command.
        #[arg(long = "field", value_name = "NAME=VALUE")]
        fields: Vec<String>,

        /// Clear every optional registered field this command gives no value
        /// for. A required field is prompted for either way.
        #[arg(long)]
        clear_omitted: bool,
    },

    /// Remove an app.
    Remove {
        /// A provider named by `moat app list`.
        provider: String,
        /// The key it was registered under.
        key: String,
    },
}

pub async fn run(
    moat: &MoatClient,
    action: ProviderAppAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ProviderAppAction::List { provider } => list(moat, provider.as_deref(), json).await,
        ProviderAppAction::Spec { provider } => {
            let spec = spec(moat, &provider).await?;
            output::emit(json, &spec, || print_spec(&spec));
            Ok(())
        }
        ProviderAppAction::Set {
            provider,
            key,
            fields,
            clear_omitted,
        } => set(moat, &provider, &key, &fields, clear_omitted, json).await,
        ProviderAppAction::Remove { provider, key } => {
            let response = moat.raw_delete(&format!("{PATH}/{provider}/{key}")).await?;
            output::emit(json, &response, || {
                println!("Removed the {provider} app registered as '{key}'.");
            });
            Ok(())
        }
    }
}

async fn list(moat: &MoatClient, provider: Option<&str>, json: bool) -> Result<(), ClientError> {
    let response = fetch(moat).await?;
    if json {
        output::print_json_typed(&response);
        return Ok(());
    }
    for spec in response
        .specs
        .iter()
        .filter(|spec| provider.is_none_or(|name| name == spec.provider))
    {
        let registered: Vec<_> = response
            .apps
            .iter()
            .filter(|app| app.provider == spec.provider)
            .collect();
        println!("{}  ({})", spec.provider, spec.display_name);
        if registered.is_empty() {
            println!(
                "  none registered — `moat app set {} <{}>`",
                spec.provider, spec.key_label
            );
        }
        for app in registered {
            println!("  {}  updated {}", app.key, app.updated_at);
            for (name, value) in &app.config {
                println!("    {name} = {value}");
            }
            for name in app.secret_fingerprints.keys() {
                println!("    {name} = (stored)");
            }
        }
    }
    Ok(())
}

async fn set(
    moat: &MoatClient,
    provider: &str,
    key: &str,
    given: &[String],
    clear_omitted: bool,
    json: bool,
) -> Result<(), ClientError> {
    let response = fetch(moat).await?;
    let spec = pick_spec(&response, provider)?;
    let mut fields = BTreeMap::new();
    for field in given {
        let (name, value) = field.split_once('=').ok_or_else(|| {
            ClientError::Other(format!("--field takes name=value, not '{field}'"))
        })?;
        // Checked here as well as on the server, so a typo costs no round trip
        // and the reply names the fields this provider actually takes.
        if spec.field(name).is_none() {
            return Err(ClientError::Other(format!(
                "'{name}' is not a field of the {} app. Fields: {}",
                spec.display_name,
                field_names(&spec)
            )));
        }
        fields.insert(name.to_string(), read_field(name, value)?);
    }
    for field in &spec.fields {
        if field.required && !fields.contains_key(&field.name) {
            fields.insert(field.name.clone(), prompt(&field.label, field.secret)?);
        }
    }

    let registered = response
        .apps
        .iter()
        .find(|app| app.provider == provider && app.key == key);
    if let Some(app) = registered.filter(|_| !clear_omitted) {
        let dropped = secrets_that_would_be_cleared(&spec, app, &fields);
        if !dropped.is_empty() {
            return Err(ClientError::Other(format!(
                "Saving replaces every field of the {} app, and these are stored today but this \
                 command gives no value for them: {}. Give each one again, or pass \
                 --clear-omitted to drop them.",
                spec.display_name,
                dropped.join(", ")
            )));
        }
        carry_forward(&spec, app, &mut fields);
    }

    let body = serde_json::json!({ "fields": fields });
    let response = moat
        .raw_put(&format!("{PATH}/{provider}/{key}"), &body)
        .await?;
    if json {
        output::print_json_typed(&response);
        return Ok(());
    }
    println!(
        "Registered the {} app for '{}'.",
        spec.display_name,
        response["key"].as_str().unwrap_or(key)
    );
    if let Some(callback) = &spec.callback_url {
        println!("Its redirect URI must be exactly: {callback}");
    }
    Ok(())
}

async fn fetch(moat: &MoatClient) -> Result<ProviderAppsResponse, ClientError> {
    let response = moat.raw_get(PATH).await?;
    serde_json::from_value(response).map_err(|e| {
        ClientError::Other(format!(
            "{PATH} answered something this CLI cannot read ({e}). A backend older than this \
             command does not serve it — use `moat github-app` or the per-provider endpoints."
        ))
    })
}

async fn spec(moat: &MoatClient, provider: &str) -> Result<ProviderAppSpec, ClientError> {
    pick_spec(&fetch(moat).await?, provider)
}

fn pick_spec(
    response: &ProviderAppsResponse,
    provider: &str,
) -> Result<ProviderAppSpec, ClientError> {
    response
        .specs
        .iter()
        .find(|spec| spec.provider == provider)
        .cloned()
        .ok_or_else(|| {
            let known = response
                .specs
                .iter()
                .map(|spec| spec.provider.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            ClientError::Other(format!(
                "'{provider}' is not a provider this deployment registers apps for. Known: {known}"
            ))
        })
}

/// The secret fields `app` has stored that `fields` gives no value for.
///
/// A save replaces every field, so each one named here would be cleared — a
/// secret can't be read back, so this can't be resent the way [`carry_forward`]
/// resends a plain value.
fn secrets_that_would_be_cleared(
    spec: &ProviderAppSpec,
    app: &ProviderAppSummary,
    fields: &BTreeMap<String, String>,
) -> Vec<String> {
    spec.fields
        .iter()
        .filter(|field| field.secret)
        .filter(|field| !fields.contains_key(&field.name))
        .filter(|field| app.secret_fingerprints.contains_key(&field.name))
        .map(|field| field.name.clone())
        .collect()
}

/// Add every registered plain value `fields` gives no value for, so changing
/// one field of an app does not clear the rest.
fn carry_forward(
    spec: &ProviderAppSpec,
    app: &ProviderAppSummary,
    fields: &mut BTreeMap<String, String>,
) {
    for field in spec.fields.iter().filter(|field| !field.secret) {
        if fields.contains_key(&field.name) {
            continue;
        }
        if let Some(value) = app.config.get(&field.name) {
            fields.insert(field.name.clone(), value.clone());
        }
    }
}

fn print_spec(spec: &ProviderAppSpec) {
    println!("{}  ({})", spec.provider, spec.display_name);
    println!("  key: {} — {}", spec.key_label, spec.key_description);
    if let Some(callback) = &spec.callback_url {
        println!("  redirect URI to register at the provider: {callback}");
    }
    for field in &spec.fields {
        let requirement = if field.required {
            "required"
        } else {
            "optional"
        };
        let secrecy = if field.secret { ", secret" } else { "" };
        println!("  {} ({requirement}{secrecy})", field.name);
        println!("    {} — {}", field.label, field.description);
    }
}

fn field_names(spec: &ProviderAppSpec) -> String {
    spec.fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A field value given inline, as `@path`, or as `-` for standard input.
fn read_field(name: &str, value: &str) -> Result<String, ClientError> {
    if value == "-" {
        let mut read = String::new();
        std::io::stdin()
            .read_to_string(&mut read)
            .map_err(|e| ClientError::Other(format!("Could not read {name} from stdin: {e}")))?;
        return Ok(read);
    }
    match value.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| ClientError::Other(format!("Could not read {name} from {path}: {e}"))),
        None => Ok(value.to_string()),
    }
}

fn prompt(label: &str, secret: bool) -> Result<String, ClientError> {
    let read = if secret {
        dialoguer::Password::new().with_prompt(label).interact()
    } else {
        dialoguer::Input::<String>::new()
            .with_prompt(label)
            .interact_text()
    };
    read.map_err(|e| ClientError::Other(format!("Could not read {label}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_api_schemas::provider_apps::ProviderAppField;
    use std::io::Write;

    fn field(name: &str, secret: bool) -> ProviderAppField {
        ProviderAppField {
            name: name.to_string(),
            label: name.to_string(),
            description: String::new(),
            secret,
            required: false,
        }
    }

    fn linear_spec() -> ProviderAppSpec {
        ProviderAppSpec {
            provider: "linear".to_string(),
            display_name: "Linear".to_string(),
            key_label: "app key".to_string(),
            key_description: String::new(),
            callback_url: None,
            fields: vec![
                field("webhookSecret", true),
                field("oauthClientId", true),
                field("apiBaseUrl", false),
            ],
        }
    }

    fn linear_app() -> ProviderAppSummary {
        ProviderAppSummary {
            provider: "linear".to_string(),
            key: "default".to_string(),
            display_name: "Linear".to_string(),
            config: BTreeMap::from([(
                "apiBaseUrl".to_string(),
                "https://linear.example/graphql".to_string(),
            )]),
            secret_fingerprints: BTreeMap::from([
                ("webhookSecret".to_string(), "abc123".to_string()),
                ("oauthClientId".to_string(), "def456".to_string()),
            ]),
            updated_at: "2026-09-10T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn a_stored_secret_is_not_dropped_by_a_command_that_omits_it() {
        // A save replaces every field, so `--field apiBaseUrl=…` alone would
        // clear both Linear secrets. Nothing about Linear is required, so
        // nothing prompts and only this stops it.
        let dropped = secrets_that_would_be_cleared(
            &linear_spec(),
            &linear_app(),
            &BTreeMap::from([(
                "apiBaseUrl".to_string(),
                "https://other.example".to_string(),
            )]),
        );
        assert_eq!(dropped, vec!["webhookSecret", "oauthClientId"]);

        let given = BTreeMap::from([
            ("webhookSecret".to_string(), "shh".to_string()),
            ("oauthClientId".to_string(), "id".to_string()),
        ]);
        assert!(secrets_that_would_be_cleared(&linear_spec(), &linear_app(), &given).is_empty());
    }

    #[test]
    fn a_registered_plain_value_is_sent_again_rather_than_cleared() {
        let mut fields = BTreeMap::from([("webhookSecret".to_string(), "shh".to_string())]);
        carry_forward(&linear_spec(), &linear_app(), &mut fields);
        assert_eq!(
            fields.get("apiBaseUrl"),
            Some(&"https://linear.example/graphql".to_string())
        );

        // What the command did give wins over what is registered.
        let mut fields = BTreeMap::from([("apiBaseUrl".to_string(), "https://new".to_string())]);
        carry_forward(&linear_spec(), &linear_app(), &mut fields);
        assert_eq!(fields.get("apiBaseUrl"), Some(&"https://new".to_string()));
    }

    #[test]
    fn a_field_reads_from_a_file_or_inline() {
        let mut file = tempfile::NamedTempFile::new().expect("a temp file");
        writeln!(file, "gto_from_a_file").expect("a written secret");
        let path = file.path().display().to_string();

        assert_eq!(
            read_field("clientSecret", &format!("@{path}")).expect("the file is readable"),
            "gto_from_a_file\n"
        );
        assert_eq!(
            read_field("clientId", "Iv1.inline").expect("an inline value"),
            "Iv1.inline"
        );
        // The trailing newline a file carries is trimmed server-side, so both
        // routes store the same value.
    }
}

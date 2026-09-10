//! Provider app administration — `moat app list|spec|set|remove`.
//!
//! The backend publishes what each provider needs, so `--field name=value`
//! takes whatever the deployment's specs name and this command needs no edit
//! when a provider is added.
//!
//! A secret is never taken as an inline argument by default: a value passed on
//! a command line lands in shell history and in the process table. Omit it and
//! it is prompted for, or read it from a file with `@path`.

use std::collections::BTreeMap;
use std::io::Read;

use clap::Subcommand;
use moatless_api_schemas::provider_apps::{ProviderAppSpec, ProviderAppsResponse};

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
    /// A required secret left out is prompted for. The provider is asked to
    /// confirm the credentials where it can, and nothing is stored when it
    /// refuses them.
    Set {
        /// A provider named by `moat app list`, e.g. `forgejo`.
        provider: String,

        /// What the app is keyed by: a Forgejo host, or an adapter's app key.
        key: String,

        /// `name=value`, repeated once per field. `name=@path` reads the value
        /// from a file and `name=-` reads it from standard input.
        #[arg(long = "field", value_name = "NAME=VALUE")]
        fields: Vec<String>,
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
            if json {
                output::print_json_typed(&spec);
            } else {
                print_spec(&spec);
            }
            Ok(())
        }
        ProviderAppAction::Set {
            provider,
            key,
            fields,
        } => set(moat, &provider, &key, &fields, json).await,
        ProviderAppAction::Remove { provider, key } => {
            moat.raw_delete(&format!("{PATH}/{provider}/{key}")).await?;
            println!("Removed the {provider} app registered as '{key}'.");
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
    json: bool,
) -> Result<(), ClientError> {
    let spec = spec(moat, provider).await?;
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
    let response = fetch(moat).await?;
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
    use std::io::Write;

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

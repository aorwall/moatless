//! Secret management commands (`moat secret ...`).

use std::fmt::Write as _;
use std::io::{IsTerminal, Read};

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum SecretAction {
    /// List secret metadata for a scope
    List {
        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope; defaults to current user for user
        /// scope)
        #[arg(long)]
        scope_id: Option<String>,
    },

    /// Create or update a secret
    Set {
        /// Secret key (must match ^[A-Z_][A-Z0-9_]*$)
        key: String,

        /// Secret value (use "-" to read from stdin)
        #[arg(long, short = 'v')]
        value: Option<String>,

        /// Read the secret value from a file (handy for multi-line `ssh_key`
        /// PEM files)
        #[arg(long, conflicts_with = "value")]
        value_file: Option<String>,

        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope)
        #[arg(long)]
        scope_id: Option<String>,

        /// Target: agent, all, `all_servers`, backend, or server:{name}
        #[arg(long, short = 't', default_value = "agent")]
        target: String,

        /// Secret kind: env, `provider_token`, `runtime_file`, deployment,
        /// `ssh_key`
        #[arg(long, short = 'k', default_value = "env")]
        kind: String,

        /// Provider name (optional, for `provider_token` kind)
        #[arg(long)]
        provider: Option<String>,

        /// External reference (mutually exclusive with value)
        #[arg(long)]
        external_ref: Option<String>,
    },

    /// Get metadata for a secret by key
    Get {
        /// Secret key
        key: String,

        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope)
        #[arg(long)]
        scope_id: Option<String>,

        /// Target to match (defaults to any)
        #[arg(long, short = 't')]
        target: Option<String>,
    },

    /// Remove a secret by key
    Remove {
        /// Secret key to remove
        key: String,

        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope)
        #[arg(long)]
        scope_id: Option<String>,

        /// Target to match (required if multiple secrets share the same key)
        #[arg(long, short = 't')]
        target: Option<String>,
    },

    /// Disable a secret (skipped during resolution; keeps its value)
    Disable {
        /// Secret key to disable
        key: String,

        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope)
        #[arg(long)]
        scope_id: Option<String>,

        /// Target to match (required if multiple secrets share the same key)
        #[arg(long, short = 't')]
        target: Option<String>,
    },

    /// Enable a previously disabled secret
    Enable {
        /// Secret key to enable
        key: String,

        /// Secret scope: user, team, or global
        #[arg(long, short = 's', default_value = "user")]
        scope: String,

        /// Scope ID (team ID for team scope)
        #[arg(long)]
        scope_id: Option<String>,

        /// Target to match (required if multiple secrets share the same key)
        #[arg(long, short = 't')]
        target: Option<String>,
    },
}

pub async fn run(client: &MoatClient, action: SecretAction, json: bool) -> Result<(), ClientError> {
    match action {
        SecretAction::List { scope, scope_id } => {
            list_secrets(client, &scope, scope_id.as_deref(), json).await
        }
        SecretAction::Set {
            key,
            value,
            value_file,
            scope,
            scope_id,
            target,
            kind,
            provider,
            external_ref,
        } => {
            set_secret(
                client,
                &key,
                value,
                value_file.as_deref(),
                &scope,
                scope_id.as_deref(),
                &target,
                &kind,
                provider.as_deref(),
                external_ref.as_deref(),
                json,
            )
            .await
        }
        SecretAction::Get {
            key,
            scope,
            scope_id,
            target,
        } => {
            get_secret(
                client,
                &key,
                &scope,
                scope_id.as_deref(),
                target.as_deref(),
                json,
            )
            .await
        }
        SecretAction::Remove {
            key,
            scope,
            scope_id,
            target,
        } => {
            remove_secret(
                client,
                &key,
                &scope,
                scope_id.as_deref(),
                target.as_deref(),
                json,
            )
            .await
        }
        SecretAction::Disable {
            key,
            scope,
            scope_id,
            target,
        } => {
            set_enabled(
                client,
                &key,
                &scope,
                scope_id.as_deref(),
                target.as_deref(),
                false,
                json,
            )
            .await
        }
        SecretAction::Enable {
            key,
            scope,
            scope_id,
            target,
        } => {
            set_enabled(
                client,
                &key,
                &scope,
                scope_id.as_deref(),
                target.as_deref(),
                true,
                json,
            )
            .await
        }
    }
}

async fn list_secrets(
    client: &MoatClient,
    scope: &str,
    scope_id: Option<&str>,
    json: bool,
) -> Result<(), ClientError> {
    validate_scope(scope)?;

    let mut path = format!("/api/v1/secrets?scope={scope}");
    if let Some(id) = scope_id {
        let _ = write!(path, "&scopeId={id}");
    }

    let resp = client.raw_get(&path).await?;

    if json {
        output::print_json_typed(&resp);
    } else {
        let empty = vec![];
        let secrets = resp.as_array().unwrap_or(&empty);
        if secrets.is_empty() {
            println!("No secrets found.");
            return Ok(());
        }

        let headers = &["KEY", "TARGET", "KIND", "SOURCE", "STATE", "UPDATED"];
        let rows: Vec<Vec<String>> = secrets
            .iter()
            .map(|s| {
                vec![
                    s.get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string(),
                    s.get("target")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string(),
                    s.get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string(),
                    s.get("source")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string(),
                    match s.get("enabled").and_then(serde_json::Value::as_bool) {
                        Some(false) => "disabled".to_string(),
                        _ => "enabled".to_string(),
                    },
                    format_timestamp(s.get("updatedAt").and_then(|v| v.as_str())),
                ]
            })
            .collect();

        output::print_table(headers, &rows);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn set_secret(
    client: &MoatClient,
    key: &str,
    value: Option<String>,
    value_file: Option<&str>,
    scope: &str,
    scope_id: Option<&str>,
    target: &str,
    kind: &str,
    provider: Option<&str>,
    external_ref: Option<&str>,
    json: bool,
) -> Result<(), ClientError> {
    validate_scope(scope)?;

    let resolved_value = match value_file {
        Some(path) => Some(read_value_file(path)?),
        None => read_value(value)?,
    };

    if resolved_value.is_none() && external_ref.is_none() {
        return Err(ClientError::Other(
            "Either --value or --external-ref is required".into(),
        ));
    }
    if resolved_value.is_some() && external_ref.is_some() {
        return Err(ClientError::Other(
            "--value and --external-ref are mutually exclusive".into(),
        ));
    }

    let mut body = serde_json::json!({
        "scope": scope,
        "key": key,
        "kind": kind,
        "target": target,
    });

    if let Some(id) = scope_id {
        body["scopeId"] = serde_json::Value::String(id.to_string());
    }
    if let Some(v) = resolved_value {
        body["value"] = serde_json::Value::String(v);
    }
    if let Some(r) = external_ref {
        body["externalRef"] = serde_json::Value::String(r.to_string());
    }
    if let Some(p) = provider {
        body["provider"] = serde_json::Value::String(p.to_string());
    }

    let resp = client.raw_put("/api/v1/secrets", &body).await?;

    output::emit(json, &resp, || {
        println!("Secret {key} saved (scope: {scope}, target: {target})");
    });

    Ok(())
}

async fn get_secret(
    client: &MoatClient,
    key: &str,
    scope: &str,
    scope_id: Option<&str>,
    target: Option<&str>,
    json: bool,
) -> Result<(), ClientError> {
    let secrets = find_secrets_by_key(client, key, scope, scope_id, target).await?;

    if secrets.is_empty() {
        return Err(ClientError::Other(format!("Secret '{key}' not found")));
    }

    if json {
        if secrets.len() == 1 {
            output::print_json_typed(&secrets[0]);
        } else {
            output::print_json_typed(&secrets);
        }
    } else {
        for secret in &secrets {
            print_secret_detail(secret);
            if secrets.len() > 1 {
                println!();
            }
        }
    }

    Ok(())
}

async fn remove_secret(
    client: &MoatClient,
    key: &str,
    scope: &str,
    scope_id: Option<&str>,
    target: Option<&str>,
    json: bool,
) -> Result<(), ClientError> {
    let secrets = find_secrets_by_key(client, key, scope, scope_id, target).await?;

    if secrets.is_empty() {
        return Err(ClientError::Other(format!("Secret '{key}' not found")));
    }

    if secrets.len() > 1 && target.is_none() {
        let targets: Vec<&str> = secrets
            .iter()
            .filter_map(|s| s.get("target").and_then(|v| v.as_str()))
            .collect();
        return Err(ClientError::Other(format!(
            "Multiple secrets match key '{key}' with targets: {}. Use --target to specify.",
            targets.join(", ")
        )));
    }

    let secret = &secrets[0];
    let id = secret
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ClientError::Other("Secret missing id".into()))?;

    let resp = client.raw_delete(&format!("/api/v1/secrets/{id}")).await?;

    output::emit(json, &resp, || println!("Secret {key} removed"));

    Ok(())
}

/// Toggle a secret's `enabled` flag via PATCH without touching its value.
async fn set_enabled(
    client: &MoatClient,
    key: &str,
    scope: &str,
    scope_id: Option<&str>,
    target: Option<&str>,
    enabled: bool,
    json: bool,
) -> Result<(), ClientError> {
    let secrets = find_secrets_by_key(client, key, scope, scope_id, target).await?;

    if secrets.is_empty() {
        return Err(ClientError::Other(format!("Secret '{key}' not found")));
    }

    if secrets.len() > 1 && target.is_none() {
        let targets: Vec<&str> = secrets
            .iter()
            .filter_map(|s| s.get("target").and_then(|v| v.as_str()))
            .collect();
        return Err(ClientError::Other(format!(
            "Multiple secrets match key '{key}' with targets: {}. Use --target to specify.",
            targets.join(", ")
        )));
    }

    let secret = &secrets[0];
    let id = secret
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ClientError::Other("Secret missing id".into()))?;

    let resp = client
        .raw_patch(
            &format!("/api/v1/secrets/{id}"),
            &serde_json::json!({ "enabled": enabled }),
        )
        .await?;

    output::emit(json, &resp, || {
        let state = if enabled { "enabled" } else { "disabled" };
        println!("Secret {key} {state}");
    });

    Ok(())
}

// Helper functions

fn validate_scope(scope: &str) -> Result<(), ClientError> {
    match scope {
        "user" | "team" | "global" => Ok(()),
        other => Err(ClientError::Other(format!(
            "Invalid scope '{other}'. Must be one of: user, team, global"
        ))),
    }
}

fn read_value(value: Option<String>) -> Result<Option<String>, ClientError> {
    match value.as_deref() {
        Some("-") => {
            if std::io::stdin().is_terminal() {
                return Err(ClientError::Other(
                    "Cannot read from stdin in interactive mode. Pipe input or use --value directly.".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| ClientError::Other(format!("Failed to read from stdin: {e}")))?;
            Ok(Some(buf.trim_end_matches('\n').to_string()))
        }
        Some(v) => Ok(Some(v.to_string())),
        None => Ok(None),
    }
}

fn read_value_file(path: &str) -> Result<String, ClientError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| ClientError::Other(format!("Failed to read value file '{path}': {e}")))?;
    // Preserve the file's internal content but drop a single trailing newline;
    // the sandbox re-adds one when materializing ssh_key secrets to ~/.ssh.
    Ok(contents.trim_end_matches('\n').to_string())
}

async fn find_secrets_by_key(
    client: &MoatClient,
    key: &str,
    scope: &str,
    scope_id: Option<&str>,
    target: Option<&str>,
) -> Result<Vec<serde_json::Value>, ClientError> {
    validate_scope(scope)?;

    let mut path = format!("/api/v1/secrets?scope={scope}");
    if let Some(id) = scope_id {
        let _ = write!(path, "&scopeId={id}");
    }

    let resp = client.raw_get(&path).await?;
    let empty = vec![];
    let secrets = resp.as_array().unwrap_or(&empty);

    let filtered: Vec<serde_json::Value> = secrets
        .iter()
        .filter(|s| {
            let key_matches = s.get("key").and_then(|v| v.as_str()) == Some(key);
            let target_matches = match target {
                Some(t) => s.get("target").and_then(|v| v.as_str()) == Some(t),
                None => true,
            };
            key_matches && target_matches
        })
        .cloned()
        .collect();

    Ok(filtered)
}

fn print_secret_detail(secret: &serde_json::Value) {
    let get_str = |key: &str| -> &str { secret.get(key).and_then(|v| v.as_str()).unwrap_or("-") };

    println!("ID:         {}", get_str("id"));
    println!("Key:        {}", get_str("key"));
    println!("Scope:      {}", get_str("scopeType"));
    if let Some(scope_id) = secret.get("scopeId").and_then(|v| v.as_str()) {
        println!("Scope ID:   {scope_id}");
    }
    println!("Target:     {}", get_str("target"));
    println!("Kind:       {}", get_str("kind"));
    if let Some(provider) = secret.get("provider").and_then(|v| v.as_str()) {
        println!("Provider:   {provider}");
    }
    println!("Source:     {}", get_str("source"));
    println!(
        "State:      {}",
        match secret.get("enabled").and_then(serde_json::Value::as_bool) {
            Some(false) => "disabled",
            _ => "enabled",
        }
    );
    println!(
        "Version:    {}",
        secret
            .get("version")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    );
    println!(
        "Updated:    {}",
        format_timestamp(secret.get("updatedAt").and_then(|v| v.as_str()))
    );
    if let Some(last_used) = secret.get("lastUsedAt").and_then(|v| v.as_str()) {
        println!("Last used:  {}", format_timestamp(Some(last_used)));
    }
}

fn format_timestamp(ts: Option<&str>) -> String {
    ts.map_or_else(
        || "-".to_string(),
        |s| {
            if s.len() > 19 {
                s[..19].replace('T', " ")
            } else {
                s.to_string()
            }
        },
    )
}

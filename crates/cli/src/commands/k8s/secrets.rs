//! `moat k8s secrets ...` — manage the cluster Secret that holds app credentials.
//!
//! Collection priority: existing cluster Secret → discovered `.env` files →
//! interactive masked prompts. Process environment is **not** scanned.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use clap::Subcommand;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::helpers::{
    kubectl_json, kubectl_stdin, DEFAULT_NAMESPACE, PULL_SECRET_NAME, SECRET_NAME,
};
use crate::client::ClientError;
use crate::output;

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Collect app secrets and apply them as `vibe-secrets` in the cluster
    Bootstrap(BootstrapArgs),

    /// Show which keys are set in the cluster (values masked)
    List(ScopedArgs),

    /// Set a single secret key (prompts for the value)
    Set(SetArgs),

    /// Create the `ghcr-secret` image-pull Secret for GHCR
    PullToken(PullTokenArgs),
}

#[derive(clap::Args)]
pub struct BootstrapArgs {
    /// Namespace for the Secret
    #[arg(long, short = 'n', default_value = DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// Path to a values file — used to scope which keys are prompted for
    #[arg(long, default_value = super::helpers::DEFAULT_VALUES_FILE)]
    pub values: String,

    /// Read known keys from this file instead of auto-discovering `.env*`
    #[arg(long)]
    pub env_file: Option<String>,

    /// Skip `.env*` auto-discovery
    #[arg(long)]
    pub no_env_file: bool,

    /// Re-prompt for keys that already exist in the cluster Secret
    #[arg(long)]
    pub overwrite: bool,

    /// Print the Secret YAML to stdout and exit without applying
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub struct ScopedArgs {
    #[arg(long, short = 'n', default_value = DEFAULT_NAMESPACE)]
    pub namespace: String,
}

#[derive(clap::Args)]
pub struct SetArgs {
    #[arg(long, short = 'n', default_value = DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// The key to set (e.g. ANTHROPIC_API_KEY)
    pub key: String,
}

#[derive(clap::Args)]
pub struct PullTokenArgs {
    #[arg(long, short = 'n', default_value = DEFAULT_NAMESPACE)]
    pub namespace: String,

    /// Your GitHub username (defaults to prompt)
    #[arg(long)]
    pub username: Option<String>,

    /// The GitHub PAT with `read:packages` scope (prompts if omitted)
    #[arg(long)]
    pub token: Option<String>,
}

pub async fn run(action: SecretsAction, json: bool) -> Result<(), ClientError> {
    match action {
        SecretsAction::Bootstrap(args) => bootstrap(args, json).await,
        SecretsAction::List(args) => list(args, json).await,
        SecretsAction::Set(args) => set_single(args, json).await,
        SecretsAction::PullToken(args) => pull_token(args, json).await,
    }
}

// ---------------------------------------------------------------------------
// Key catalog — which keys to ask for, given what values.local.yaml enables.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct KeySpec {
    key: &'static str,
    /// Whether the install fails without this key.
    required: bool,
    /// One-line hint shown next to the prompt.
    hint: &'static str,
    /// Whether the value should be masked when prompting / displaying.
    secret: bool,
}

const CORE_KEYS: &[KeySpec] = &[
    KeySpec {
        key: "DATABASE_URL",
        required: true,
        hint: "PostgreSQL URL, e.g. postgres://user:pass@host:5432/vibe",
        secret: true,
    },
    KeySpec {
        key: "GITHUB_TOKEN",
        required: false,
        hint: "GitHub PAT used for repo clones (read:user, repo)",
        secret: true,
    },
    KeySpec {
        key: "SANDBOX_API_KEY",
        required: false,
        hint: "Leave blank to auto-generate",
        secret: true,
    },
];

const EXTERNAL_REDIS_KEY: KeySpec = KeySpec {
    key: "REDIS_URL",
    required: true,
    hint: "External Redis URL (only when redis.enabled=false in values)",
    secret: true,
};

const GITHUB_AUTH_KEYS: &[KeySpec] = &[
    KeySpec {
        key: "GITHUB_CLIENT_ID",
        required: true,
        hint: "OAuth App Client ID",
        secret: false,
    },
    KeySpec {
        key: "GITHUB_CLIENT_SECRET",
        required: true,
        hint: "OAuth App Client Secret",
        secret: true,
    },
];

const AUTH0_KEYS: &[KeySpec] = &[
    KeySpec {
        key: "AUTH0_DOMAIN",
        required: true,
        hint: "your-tenant.auth0.com",
        secret: false,
    },
    KeySpec {
        key: "AUTH0_CLIENT_ID",
        required: true,
        hint: "Auth0 Application Client ID",
        secret: false,
    },
    KeySpec {
        key: "AUTH0_CLIENT_SECRET",
        required: true,
        hint: "Auth0 Application Client Secret",
        secret: true,
    },
];

fn keys_for_values(values: Option<&serde_yaml::Value>) -> Vec<&'static KeySpec> {
    let mut keys: Vec<&'static KeySpec> = CORE_KEYS.iter().collect();

    // External Redis → REDIS_URL required. Bundled Redis (the default) → no key needed.
    let redis_enabled = values
        .and_then(|v| v.get("redis"))
        .and_then(|r| r.get("enabled"))
        .and_then(serde_yaml::Value::as_bool)
        .unwrap_or(true);
    if !redis_enabled {
        keys.push(&EXTERNAL_REDIS_KEY);
    }

    let auth_provider = values
        .and_then(|v| v.get("auth"))
        .and_then(|a| a.get("provider"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    match auth_provider {
        "github" => keys.extend(GITHUB_AUTH_KEYS.iter()),
        "auth0" => keys.extend(AUTH0_KEYS.iter()),
        _ => {}
    }
    keys
}

// ---------------------------------------------------------------------------
// Bootstrap: the headline command.
// ---------------------------------------------------------------------------

async fn bootstrap(args: BootstrapArgs, json_out: bool) -> Result<(), ClientError> {
    // Step 1: load values file (optional).
    let values: Option<serde_yaml::Value> = std::fs::read_to_string(&args.values)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok());

    if values.is_none() {
        eprintln!(
            "Note: {} not found — prompting for core keys only. Run `moat k8s init` first for auth-specific prompts.",
            args.values
        );
    }

    let specs = keys_for_values(values.as_ref());

    // Step 2: snapshot existing cluster secret.
    let existing = load_cluster_secret(&args.namespace, SECRET_NAME)
        .await
        .unwrap_or_default();

    // Step 3: discover .env files (unless skipped).
    let env_values = if args.no_env_file {
        BTreeMap::new()
    } else if let Some(path) = args.env_file.as_deref() {
        read_env_file(Path::new(path))?
    } else {
        discover_env(&specs)?
    };

    // Step 4: gather final values.
    let mut final_values: BTreeMap<String, String> = BTreeMap::new();
    let mut sources: BTreeMap<String, &'static str> = BTreeMap::new();

    for spec in &specs {
        let key = spec.key;
        if !args.overwrite {
            if let Some(existing_val) = existing.get(key) {
                if !existing_val.is_empty() {
                    sources.insert(key.to_string(), "cluster");
                    // Keep existing; don't include in manifest patch.
                    continue;
                }
            }
        }

        if let Some(from_env) = env_values.get(key) {
            if !from_env.is_empty() {
                final_values.insert(key.to_string(), from_env.clone());
                sources.insert(key.to_string(), "env-file");
                continue;
            }
        }

        // Interactive prompt — mask secrets, regular input for public values.
        if !std::io::stdin().is_terminal() {
            if spec.required {
                return Err(ClientError::Other(format!(
                    "{key} not set in cluster or env file, and stdin is not a terminal. Pass --env-file or run interactively."
                )));
            }
            continue;
        }
        if !spec.required {
            let ask = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!("Set optional {key}? ({})", spec.hint))
                .default(false)
                .interact()
                .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
            if !ask {
                continue;
            }
        }
        let value = prompt_value(spec)?;
        if !value.is_empty() {
            final_values.insert(key.to_string(), value);
            sources.insert(key.to_string(), "prompt");
        }
    }

    // Step 5: build + emit Secret manifest.
    let combined = {
        let mut m = existing.clone();
        m.extend(final_values.clone());
        m
    };
    let manifest = render_secret_manifest(&args.namespace, SECRET_NAME, &combined);

    if args.dry_run {
        println!("{manifest}");
        return Ok(());
    }

    apply_manifest(&manifest).await?;

    if json_out {
        output::print_json_typed(&json!({
            "applied": true,
            "namespace": args.namespace,
            "secret": SECRET_NAME,
            "keys": combined.keys().collect::<Vec<_>>(),
            "changedKeys": final_values.keys().collect::<Vec<_>>(),
            "sources": sources,
        }));
    } else {
        println!();
        println!(
            "Applied Secret/{SECRET_NAME} to namespace {}.",
            args.namespace
        );
        let headers = ["KEY", "SOURCE"];
        let rows: Vec<Vec<String>> = specs
            .iter()
            .map(|s| {
                vec![
                    s.key.to_string(),
                    sources
                        .get(s.key)
                        .copied()
                        .unwrap_or(if combined.contains_key(s.key) {
                            "existing"
                        } else {
                            "missing"
                        })
                        .to_string(),
                ]
            })
            .collect();
        output::print_table(&headers, &rows);
    }

    Ok(())
}

fn prompt_value(spec: &KeySpec) -> Result<String, ClientError> {
    let prompt = format!("{} ({})", spec.key, spec.hint);
    let theme = ColorfulTheme::default();
    let value = if spec.secret {
        Password::with_theme(&theme)
            .with_prompt(prompt)
            .allow_empty_password(!spec.required)
            .interact()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?
    } else {
        Input::<String>::with_theme(&theme)
            .with_prompt(prompt)
            .allow_empty(!spec.required)
            .interact_text()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?
    };
    Ok(value)
}

// ---------------------------------------------------------------------------
// `list`: show which keys are set in the cluster.
// ---------------------------------------------------------------------------

async fn list(args: ScopedArgs, json_out: bool) -> Result<(), ClientError> {
    let existing = load_cluster_secret(&args.namespace, SECRET_NAME).await?;
    if json_out {
        let keys: Vec<&String> = existing.keys().collect();
        output::print_json_typed(&json!({ "keys": keys }));
        return Ok(());
    }
    if existing.is_empty() {
        println!("Secret/{SECRET_NAME} is empty or does not exist.");
        return Ok(());
    }
    let headers = ["KEY", "LENGTH"];
    let rows: Vec<Vec<String>> = existing
        .iter()
        .map(|(k, v)| vec![k.clone(), v.len().to_string()])
        .collect();
    output::print_table(&headers, &rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// `set`: patch one key.
// ---------------------------------------------------------------------------

async fn set_single(args: SetArgs, json_out: bool) -> Result<(), ClientError> {
    let value = Password::with_theme(&ColorfulTheme::default())
        .with_prompt(&args.key)
        .interact()
        .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;

    let mut existing = load_cluster_secret(&args.namespace, SECRET_NAME)
        .await
        .unwrap_or_default();
    existing.insert(args.key.clone(), value);
    let manifest = render_secret_manifest(&args.namespace, SECRET_NAME, &existing);
    apply_manifest(&manifest).await?;

    if json_out {
        output::print_json_typed(&json!({ "updated": args.key }));
    } else {
        println!("Updated {} in Secret/{SECRET_NAME}.", args.key);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `pull-token`: dockerconfigjson Secret for GHCR.
// ---------------------------------------------------------------------------

async fn pull_token(args: PullTokenArgs, json_out: bool) -> Result<(), ClientError> {
    let username = match args.username {
        Some(u) => u,
        None => Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("GitHub username")
            .interact_text()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?,
    };
    let token = match args.token {
        Some(t) => t,
        None => Password::with_theme(&ColorfulTheme::default())
            .with_prompt("GitHub PAT (needs read:packages)")
            .interact()
            .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?,
    };

    let auth = B64.encode(format!("{username}:{token}"));
    let dockercfg = json!({
        "auths": {
            "ghcr.io": {
                "username": username,
                "password": token,
                "auth": auth,
            }
        }
    })
    .to_string();
    let dockercfg_b64 = B64.encode(dockercfg);

    let manifest = format!(
        r#"apiVersion: v1
kind: Secret
type: kubernetes.io/dockerconfigjson
metadata:
  name: {name}
  namespace: {ns}
data:
  .dockerconfigjson: {data}
"#,
        name = PULL_SECRET_NAME,
        ns = args.namespace,
        data = dockercfg_b64,
    );

    apply_manifest(&manifest).await?;

    if json_out {
        output::print_json_typed(&json!({ "applied": true, "secret": PULL_SECRET_NAME }));
    } else {
        println!(
            "Applied Secret/{PULL_SECRET_NAME} (dockerconfigjson) to {}.",
            args.namespace
        );
        println!("Add `imagePullSecrets: [{{name: {PULL_SECRET_NAME}}}]` to values.local.yaml if not already set.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_cluster_secret(
    namespace: &str,
    name: &str,
) -> Result<BTreeMap<String, String>, ClientError> {
    let Some(v) = kubectl_json(&["-n", namespace, "get", "secret", name, "-o", "json"]).await?
    else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    if let Some(data) = v.get("data").and_then(|d| d.as_object()) {
        for (k, val) in data {
            let b64 = val.as_str().unwrap_or("");
            let decoded = B64
                .decode(b64)
                .map_err(|e| ClientError::Other(format!("Invalid base64 for {k}: {e}")))?;
            let s = String::from_utf8(decoded)
                .map_err(|e| ClientError::Other(format!("Non-UTF8 value for {k}: {e}")))?;
            out.insert(k.clone(), s);
        }
    }
    Ok(out)
}

fn render_secret_manifest(namespace: &str, name: &str, data: &BTreeMap<String, String>) -> String {
    let mut yaml = String::new();
    yaml.push_str("apiVersion: v1\nkind: Secret\ntype: Opaque\nmetadata:\n");
    let _ = writeln!(yaml, "  name: {name}");
    let _ = writeln!(yaml, "  namespace: {namespace}");
    yaml.push_str("data:\n");
    for (k, v) in data {
        let _ = writeln!(yaml, "  {k}: {}", B64.encode(v));
    }
    yaml
}

async fn apply_manifest(manifest: &str) -> Result<(), ClientError> {
    let out = kubectl_stdin(&["apply", "-f", "-"], manifest).await?;
    out.into_stdout("kubectl apply").map(|_| ())
}

fn discover_env(specs: &[&KeySpec]) -> Result<BTreeMap<String, String>, ClientError> {
    let candidates = [".env.local", ".env", "backend/.env"];
    let mut found = BTreeMap::new();
    for cand in candidates {
        let p = PathBuf::from(cand);
        if !p.exists() {
            continue;
        }
        let parsed = read_env_file(&p)?;
        // Only surface keys we care about.
        let relevant: Vec<&str> = specs
            .iter()
            .map(|s| s.key)
            .filter(|k| parsed.contains_key(*k))
            .collect();
        if relevant.is_empty() {
            continue;
        }
        if std::io::stdin().is_terminal() {
            let approved = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Found {} moatless key(s) in {cand}: {}. Use them?",
                    relevant.len(),
                    relevant.join(", ")
                ))
                .default(true)
                .interact()
                .map_err(|e| ClientError::Other(format!("Prompt failed: {e}")))?;
            if !approved {
                continue;
            }
        }
        for k in relevant {
            if let Some(v) = parsed.get(k) {
                found.entry(k.to_string()).or_insert_with(|| v.clone());
            }
        }
    }
    Ok(found)
}

/// Minimal `.env` parser: `KEY=VALUE` per line, `#` comments, optional surrounding quotes.
fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>, ClientError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ClientError::Other(format!("Failed to read {}: {e}", path.display())))?;
    let mut out = BTreeMap::new();
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let stripped = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, val)) = stripped.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !is_env_key(key) {
            // Silently skip lines that don't look like env keys.
            let _ = idx;
            continue;
        }
        let value = strip_quotes(val.trim());
        out.insert(key.to_string(), value);
    }
    Ok(out)
}

fn is_env_key(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn strip_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_env() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# comment\nANTHROPIC_API_KEY=sk-ant-abc\nexport DATABASE_URL=\"postgres://a:b@h/c\"\nEMPTY=\n",
        )
        .unwrap();
        let map = read_env_file(&path).unwrap();
        assert_eq!(
            map.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-ant-abc")
        );
        assert_eq!(
            map.get("DATABASE_URL").map(String::as_str),
            Some("postgres://a:b@h/c")
        );
        assert_eq!(map.get("EMPTY").map(String::as_str), Some(""));
    }

    #[test]
    fn renders_secret_manifest() {
        let mut data = BTreeMap::new();
        data.insert("K".to_string(), "v".to_string());
        let yaml = render_secret_manifest("vibe", "vibe-secrets", &data);
        assert!(yaml.contains("namespace: vibe"));
        assert!(yaml.contains("K: dg==")); // base64 of "v"
    }

    #[test]
    fn keys_for_github() {
        let v: serde_yaml::Value = serde_yaml::from_str("auth:\n  provider: github\n").unwrap();
        let keys = keys_for_values(Some(&v));
        assert!(keys.iter().any(|k| k.key == "GITHUB_CLIENT_ID"));
        assert!(!keys.iter().any(|k| k.key == "AUTH0_DOMAIN"));
        assert!(!keys.iter().any(|k| k.key == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn redis_url_only_when_external() {
        let bundled: serde_yaml::Value = serde_yaml::from_str("redis:\n  enabled: true\n").unwrap();
        assert!(!keys_for_values(Some(&bundled))
            .iter()
            .any(|k| k.key == "REDIS_URL"));

        let external: serde_yaml::Value =
            serde_yaml::from_str("redis:\n  enabled: false\n").unwrap();
        assert!(keys_for_values(Some(&external))
            .iter()
            .any(|k| k.key == "REDIS_URL"));
    }
}

//! GitHub App administration — `moat github-app list|register|rotate|remove`.
//!
//! Registering and rotating both take a signing key, and neither takes it as
//! an argument: a key passed on a command line lands in shell history and in
//! the process table. It is read from a file, or from standard input.
//!
//! No command here prints a stored key. What is shown instead is a
//! fingerprint, which is enough to tell two keys apart and reveals neither.

use std::io::Read;
use std::path::PathBuf;

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum GitHubAppAction {
    /// List every registered App, its accounts, and whether it has a bot user.
    List,

    /// Register an App created at GitHub, proving its signing key first.
    ///
    /// The key is used against GitHub immediately and the App it identifies is
    /// confirmed to be the App named by `--app-id`. Nothing is stored unless
    /// that succeeds.
    Register {
        /// Short stable name the App will be known by, e.g. `dev-bot`. This is
        /// what a bot user, an Adapter connection and a Loop refer to.
        key: String,

        /// GitHub's own numeric id for the App.
        #[arg(long)]
        app_id: u64,

        /// PEM file holding the App's signing key. Use `-` to read standard
        /// input.
        #[arg(long)]
        key_file: PathBuf,

        /// Pin the App to one installation instead of resolving one per
        /// repository owner.
        #[arg(long)]
        installation_id: Option<u64>,
    },

    /// Replace an App's signing key.
    ///
    /// The new key is proved before the stored one is replaced, so a key that
    /// GitHub will not accept leaves the App exactly as it was.
    Rotate {
        /// Name the App was registered under.
        key: String,

        /// PEM file holding the replacement key. Use `-` to read standard
        /// input.
        #[arg(long)]
        key_file: PathBuf,
    },

    /// Remove an App, installation-wide.
    ///
    /// Bot users backed by it are left in place so past work stays attributed,
    /// and are named in the output because they can no longer obtain a token.
    Remove {
        /// Name the App was registered under.
        key: String,
    },
}

pub async fn run(
    moat: &MoatClient,
    action: GitHubAppAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        GitHubAppAction::List => list(moat, json).await,
        GitHubAppAction::Register {
            key,
            app_id,
            key_file,
            installation_id,
        } => {
            let body = serde_json::json!({
                "githubAppKey": key,
                "appId": app_id,
                "privateKeyPem": read_signing_key(&key_file)?,
                "defaultInstallationId": installation_id,
            });
            let resp = moat
                .raw_post("/api/v1/admin/settings/github/apps", &body)
                .await?;
            print_registration(&resp, json, "registered");
            Ok(())
        }
        GitHubAppAction::Rotate { key, key_file } => {
            let body = serde_json::json!({ "privateKeyPem": read_signing_key(&key_file)? });
            let resp = moat
                .raw_put(
                    &format!("/api/v1/admin/settings/github/apps/{key}/key"),
                    &body,
                )
                .await?;
            print_registration(&resp, json, "rotated");
            Ok(())
        }
        GitHubAppAction::Remove { key } => {
            let resp = moat
                .raw_delete(&format!("/api/v1/admin/settings/github/apps/{key}"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Removed GitHub App '{key}'.");
                let orphaned = resp["orphanedBotUsers"].as_array().map_or(0, Vec::len);
                if orphaned > 0 {
                    println!(
                        "{orphaned} bot user(s) still name it and can no longer obtain a token:"
                    );
                    for user in resp["orphanedBotUsers"].as_array().into_iter().flatten() {
                        println!("  {}", user.as_str().unwrap_or(""));
                    }
                }
            }
            Ok(())
        }
    }
}

async fn list(moat: &MoatClient, json: bool) -> Result<(), ClientError> {
    let resp = moat.raw_get("/api/v1/admin/settings/github/apps").await?;
    if json {
        output::print_json_typed(&resp);
        return Ok(());
    }
    let apps = resp["apps"].as_array().cloned().unwrap_or_default();
    if apps.is_empty() {
        println!("No GitHub Apps are registered.");
        return Ok(());
    }
    for app in apps {
        let key = app["githubAppKey"].as_str().unwrap_or("");
        let name = app["name"].as_str().unwrap_or("");
        // The deployment-configured App is listed so admins can see it, and
        // marked because nothing here can change it.
        let origin = if app["deploymentConfigured"].as_bool().unwrap_or(false) {
            " (configured by the deployment, not editable here)"
        } else {
            ""
        };
        println!("{key}  {name}{origin}");
        if let Some(fingerprint) = app["keyFingerprint"].as_str() {
            println!("  key      {fingerprint}");
        }
        match app["botUserId"].as_str() {
            Some(id) => println!(
                "  bot      {} ({id})",
                app["botLogin"].as_str().unwrap_or("")
            ),
            None => println!(
                "  bot      none — `moat bot create --app-key {key}` creates {}",
                app["botLogin"].as_str().unwrap_or("one")
            ),
        }
        for installation in app["installations"].as_array().into_iter().flatten() {
            println!(
                "  account  {} ({})",
                installation["accountLogin"].as_str().unwrap_or(""),
                installation["installationId"].as_u64().unwrap_or(0),
            );
        }
    }
    Ok(())
}

fn print_registration(resp: &serde_json::Value, json: bool, verb: &str) {
    if json {
        output::print_json_typed(resp);
        return;
    }
    let key = resp["githubAppKey"].as_str().unwrap_or("");
    println!(
        "GitHub App '{key}' {verb}: {} (id {}, key {})",
        resp["name"].as_str().unwrap_or(""),
        resp["appId"].as_u64().unwrap_or(0),
        resp["keyFingerprint"].as_str().unwrap_or(""),
    );
    println!(
        "Its bot identity would be {}. Create it with `moat bot create --app-key {key}`.",
        resp["botLogin"].as_str().unwrap_or(""),
    );
}

/// Read a signing key from a file, or from standard input for `-`.
fn read_signing_key(path: &PathBuf) -> Result<String, ClientError> {
    if path.as_os_str() == "-" {
        let mut pem = String::new();
        std::io::stdin()
            .read_to_string(&mut pem)
            .map_err(|e| ClientError::Other(format!("Could not read the key from stdin: {e}")))?;
        return Ok(pem);
    }
    std::fs::read_to_string(path)
        .map_err(|e| ClientError::Other(format!("Could not read {}: {e}", path.display())))
}

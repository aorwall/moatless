//! Settings management commands (`moat settings ...`).

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum SettingsAction {
    /// Save a GitHub Personal Access Token
    #[command(name = "github-pat")]
    GithubPat {
        /// The GitHub PAT to save (omit to show current status)
        token: Option<String>,

        /// Remove the PAT override (restores the GitHub App user token)
        #[arg(long)]
        remove: bool,
    },

    /// Show current GitHub connection status
    #[command(name = "github-status")]
    GithubStatus,
}

pub async fn run(
    client: &MoatClient,
    action: SettingsAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        SettingsAction::GithubPat { token, remove } => {
            if remove {
                let resp = client.raw_delete("/api/v1/settings/github/pat").await?;
                output::emit(json, &resp, || println!("PAT override removed."));
            } else if let Some(pat) = token {
                let body = serde_json::json!({ "token": pat });
                let resp = client.raw_put("/api/v1/settings/github/pat", &body).await?;
                output::emit(json, &resp, || {
                    let login = resp
                        .get("login")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    println!("GitHub PAT saved. Username: {login}");
                });
            } else {
                print_github_status(client, json).await?;
            }
        }
        SettingsAction::GithubStatus => {
            print_github_status(client, json).await?;
        }
    }
    Ok(())
}

async fn print_github_status(client: &MoatClient, json: bool) -> Result<(), ClientError> {
    let resp = client.raw_get("/api/v1/settings/github/config").await?;
    output::emit(json, &resp, || {
        let connected = resp
            .get("connected")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if connected {
            let login = resp
                .get("login")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let token_type = resp
                .get("tokenType")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let has_pat = resp
                .get("hasPatOverride")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            println!("GitHub: connected");
            println!("  Username:   {login}");
            println!("  Token type: {token_type}");
            if has_pat {
                println!("  PAT override: active");
            }
        } else {
            println!("GitHub: not connected");
        }
    });
    Ok(())
}

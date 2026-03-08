//! User settings commands: get, set.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum SettingsAction {
    /// Show current user settings
    Get,
    /// Update user settings
    Set {
        /// Default repository ID for channel integrations
        #[arg(long)]
        default_repo: Option<String>,
    },
}

pub async fn run(client: &MoatClient, action: SettingsAction, json: bool) -> Result<(), ClientError> {
    match action {
        SettingsAction::Get => {
            let resp = client.get("/api/v1/users/me/settings").await?;
            if json {
                output::print_json(&resp);
            } else {
                format_settings(&resp);
            }
        }
        SettingsAction::Set { default_repo } => {
            let mut body = serde_json::Map::new();
            if let Some(repo) = default_repo {
                body.insert(
                    "defaultChannelRepositoryId".into(),
                    serde_json::json!(repo),
                );
            }
            if body.is_empty() {
                eprintln!("No settings to update. Use --default-repo <id> to set a value.");
                return Ok(());
            }
            let resp = client
                .patch(
                    "/api/v1/users/me/settings",
                    &serde_json::Value::Object(body),
                )
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                println!("Settings updated.");
                format_settings(&resp);
            }
        }
    }
    Ok(())
}

fn format_settings(resp: &serde_json::Value) {
    let default_repo = resp
        .get("defaultChannelRepositoryId")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    println!("Default channel repo: {default_repo}");
}

//! Team commands: list, get, settings get/set.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum TeamAction {
    /// List teams
    List,
    /// Get team details
    Get { team_id: String },
    /// Show team settings
    Settings {
        #[command(subcommand)]
        action: TeamSettingsAction,
    },
}

#[derive(Subcommand)]
pub enum TeamSettingsAction {
    /// Show current team settings
    Get { team_id: String },
    /// Update team settings
    Set {
        team_id: String,
        /// Default repository ID for channel integrations
        #[arg(long)]
        default_repo: Option<String>,
    },
}

pub async fn run(client: &MoatClient, action: TeamAction, json: bool) -> Result<(), ClientError> {
    match action {
        TeamAction::List => {
            let resp = client.get("/api/v1/teams").await?;
            if json {
                output::print_json(&resp);
            } else {
                format_team_list(&resp);
            }
        }
        TeamAction::Get { team_id } => {
            let resp = client.get(&format!("/api/v1/teams/{team_id}")).await?;
            if json {
                output::print_json(&resp);
            } else {
                format_team_detail(&resp);
            }
        }
        TeamAction::Settings { action } => match action {
            TeamSettingsAction::Get { team_id } => {
                let resp = client
                    .get(&format!("/api/v1/teams/{team_id}/settings"))
                    .await?;
                if json {
                    output::print_json(&resp);
                } else {
                    format_team_settings(&resp);
                }
            }
            TeamSettingsAction::Set {
                team_id,
                default_repo,
            } => {
                let mut body = serde_json::Map::new();
                if let Some(repo) = default_repo {
                    body.insert(
                        "defaultChannelRepositoryId".into(),
                        serde_json::json!(repo),
                    );
                }
                if body.is_empty() {
                    eprintln!(
                        "No settings to update. Use --default-repo <id> to set a value."
                    );
                    return Ok(());
                }
                let resp = client
                    .patch(
                        &format!("/api/v1/teams/{team_id}/settings"),
                        &serde_json::Value::Object(body),
                    )
                    .await?;
                if json {
                    output::print_json(&resp);
                } else {
                    println!("Team settings updated.");
                    format_team_settings(&resp);
                }
            }
        },
    }
    Ok(())
}

fn format_team_list(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|t| {
            vec![
                t.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                t.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                t.get("slug")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                if t.get("isDefault").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    "yes".to_string()
                } else {
                    String::new()
                },
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "SLUG", "DEFAULT"], &rows);
}

fn format_team_detail(resp: &serde_json::Value) {
    let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("-");
    let name = resp.get("name").and_then(|v| v.as_str()).unwrap_or("-");
    let slug = resp.get("slug").and_then(|v| v.as_str()).unwrap_or("-");
    let desc = resp
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let is_default = resp
        .get("isDefault")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    println!("ID:          {id}");
    println!("Name:        {name}");
    println!("Slug:        {slug}");
    println!("Description: {desc}");
    println!("Default:     {is_default}");
}

fn format_team_settings(resp: &serde_json::Value) {
    let default_repo = resp
        .get("defaultChannelRepositoryId")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    println!("Default channel repo: {default_repo}");
}

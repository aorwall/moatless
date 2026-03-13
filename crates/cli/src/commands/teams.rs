//! Team commands: list, get.

use clap::Subcommand;
use moatless_api_schemas::team::TeamResponse;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum TeamAction {
    /// List teams
    List,
    /// Get team details
    Get { team_id: String },
}

pub async fn run(client: &MoatClient, action: TeamAction, json: bool) -> Result<(), ClientError> {
    match action {
        TeamAction::List => {
            let teams = client.api().list_teams().await?;
            if json {
                output::print_json_typed(&teams);
            } else {
                format_team_list(&teams);
            }
        }
        TeamAction::Get { team_id } => {
            let team = client.api().get_team(&team_id).await?;
            if json {
                output::print_json_typed(&team);
            } else {
                format_team_detail(&team);
            }
        }
    }
    Ok(())
}

fn format_team_list(teams: &[TeamResponse]) {
    let rows: Vec<Vec<String>> = teams
        .iter()
        .map(|t| {
            vec![
                t.id.clone(),
                t.name.clone(),
                t.slug.clone(),
                if t.is_default {
                    "yes".to_string()
                } else {
                    String::new()
                },
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "SLUG", "DEFAULT"], &rows);
}

fn format_team_detail(t: &TeamResponse) {
    println!("ID:          {}", t.id);
    println!("Name:        {}", t.name);
    println!("Slug:        {}", t.slug);
    println!(
        "Description: {}",
        t.description.as_deref().unwrap_or("-")
    );
    println!("Default:     {}", t.is_default);
}

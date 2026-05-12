//! User provider ID management.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum UserAction {
    /// Associate a provider-specific user ID with this moatless user
    SetProviderId {
        /// Provider name (e.g. "slack")
        #[arg(long)]
        provider: String,
        /// Provider-specific user ID (e.g. Slack user ID "U0000000002")
        #[arg(long)]
        id: String,
    },
    /// List all provider ID mappings for this user
    GetProviderIds,
    /// Remove a provider ID mapping
    DeleteProviderId {
        /// Provider name (e.g. "slack")
        #[arg(long)]
        provider: String,
    },
}

pub async fn run(
    moat: &MoatClient,
    login: &str,
    action: UserAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        UserAction::SetProviderId { provider, id } => {
            let body = serde_json::json!({ "provider_user_id": id });
            let resp = moat
                .raw_put(
                    &format!("/api/v1/users/{login}/provider-ids/{provider}"),
                    &body,
                )
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Provider ID for '{provider}' set successfully.");
            }
        }

        UserAction::GetProviderIds => {
            let resp = moat
                .raw_get(&format!("/api/v1/users/{login}/provider-ids"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let items = resp.as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    println!("No provider ID mappings found for '{login}'.");
                } else {
                    println!("{:<12} {:<30} CREATED AT", "PROVIDER", "PROVIDER USER ID");
                    for item in &items {
                        println!(
                            "{:<12} {:<30} {}",
                            item["provider"].as_str().unwrap_or(""),
                            item["provider_user_id"].as_str().unwrap_or(""),
                            item["created_at"].as_str().unwrap_or(""),
                        );
                    }
                }
            }
        }

        UserAction::DeleteProviderId { provider } => {
            let resp = moat
                .raw_delete(&format!("/api/v1/users/{login}/provider-ids/{provider}"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Provider ID for '{provider}' deleted.");
            }
        }
    }

    Ok(())
}

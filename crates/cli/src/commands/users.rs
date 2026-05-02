//! User provider ID management and DM send commands.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum UserAction {
    /// Send a DM to the user via an external provider (e.g. Slack)
    Send {
        /// Provider to send through (e.g. "slack")
        #[arg(long)]
        provider: String,
        /// Message text
        #[arg(long)]
        text: String,
        /// Task ID to associate replies with (optional)
        #[arg(long)]
        task_id: Option<String>,
    },
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
        UserAction::Send {
            provider,
            text,
            task_id,
        } => {
            let mut body = serde_json::json!({
                "provider": provider,
                "text": text,
            });
            if let Some(ref tid) = task_id {
                body["task_id"] = serde_json::Value::String(tid.clone());
            }
            let resp = moat
                .raw_post(&format!("/api/v1/users/{login}/dm"), &body)
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!(
                    "DM sent. Channel: {}, ts: {}, subscription: {}",
                    resp["channel_id"].as_str().unwrap_or(""),
                    resp["message_ts"].as_str().unwrap_or(""),
                    resp["subscription_id"].as_str().unwrap_or("(none)"),
                );
            }
        }

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

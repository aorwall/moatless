//! Channel commands: list, create.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum ChannelAction {
    /// List channel configs
    List,
    /// Create a channel config
    Create {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        mode: String,
    },
}

pub async fn run(
    client: &MoatClient,
    action: ChannelAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ChannelAction::List => {
            let resp = client.get("/api/v1/channels/configs").await?;
            if json {
                output::print_json(&resp);
            } else {
                format_channels(&resp);
            }
        }
        ChannelAction::Create {
            provider,
            source,
            repo,
            mode,
        } => {
            let body = serde_json::json!({
                "provider": provider,
                "sourceIdentifier": source,
                "repositoryId": repo,
                "routingMode": mode,
            });
            let resp = client.post("/api/v1/channels/configs", &body).await?;
            if json {
                output::print_json(&resp);
            } else {
                let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                println!("Channel config created: {id}");
            }
        }
    }
    Ok(())
}

fn format_channels(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|c| {
            vec![
                c.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                c.get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                c.get("sourceIdentifier")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                c.get("routingMode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            ]
        })
        .collect();

    output::print_table(&["ID", "PROVIDER", "SOURCE", "MODE"], &rows);
}

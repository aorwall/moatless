//! Channel commands: list, create, defaults.

use clap::Subcommand;
use moatless_api_client::types::{
    ChannelConfigResponse, ChannelType, CreateChannelConfigRequest, RoutingMode,
};

use crate::client::{map_api_error, ClientError, MoatClient};
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
        /// Provider-specific config as JSON (e.g. '{"access_policy":"open","trigger":"all_messages"}')
        #[arg(long)]
        provider_config: Option<String>,
    },
    /// Manage default settings for a channel provider
    Defaults {
        #[command(subcommand)]
        action: ChannelDefaultsAction,
    },
}

#[derive(Subcommand)]
pub enum ChannelDefaultsAction {
    /// Get current defaults for a provider
    Get {
        #[arg(long)]
        provider: String,
    },
    /// Set defaults for a provider (creates or updates wildcard config)
    Set {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        mode: String,
        /// Provider-specific config as JSON (e.g. '{"dm_access_policy":"pairing"}')
        #[arg(long)]
        provider_config: Option<String>,
    },
}

pub async fn run(
    client: &MoatClient,
    action: ChannelAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ChannelAction::List => {
            let resp = client
                .api()
                .list_configs(None, None)
                .await
                .map_err(map_api_error)?;
            let configs = resp.into_inner();
            if json {
                output::print_json_typed(&configs);
            } else {
                format_channels(&configs);
            }
        }
        ChannelAction::Create {
            provider,
            source,
            repo,
            mode,
            provider_config,
        } => {
            let parsed_config = parse_provider_config(provider_config.as_deref())?;

            let req = CreateChannelConfigRequest {
                provider: parse_channel_type(&provider),
                source_id: source,
                repository_id: repo,
                routing_mode: parse_routing_mode(&mode),
                prompt_template: None,
                provider_config: parsed_config,
                response_config_id: None,
                scope: None,
                source_name: None,
                team_id: None,
            };
            let resp = client
                .api()
                .create_config(&req)
                .await
                .map_err(map_api_error)?;
            let created = resp.into_inner();
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Channel config created: {}", created.id);
            }
        }
        ChannelAction::Defaults { action: defaults } => {
            run_defaults(client, defaults, json).await?;
        }
    }
    Ok(())
}

async fn run_defaults(
    client: &MoatClient,
    action: ChannelDefaultsAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ChannelDefaultsAction::Get { provider } => {
            let resp = client
                .raw_get(&format!("/api/v1/channels/defaults/{provider}"))
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                format_defaults(&provider, &resp);
            }
        }
        ChannelDefaultsAction::Set {
            provider,
            repo,
            mode,
            provider_config,
        } => {
            let parsed_config = provider_config
                .as_deref()
                .map(|s| {
                    serde_json::from_str::<serde_json::Value>(s).map_err(|e| ClientError::Api {
                        status: 0,
                        body: format!("Invalid --provider-config JSON: {e}"),
                    })
                })
                .transpose()?;

            let body = serde_json::json!({
                "repositoryId": repo,
                "routingMode": mode,
                "providerConfig": parsed_config,
            });

            let resp = client
                .raw_put(&format!("/api/v1/channels/defaults/{provider}"), &body)
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Defaults saved for provider: {provider}");
                format_defaults(&provider, &resp);
            }
        }
    }
    Ok(())
}

fn format_defaults(provider: &str, v: &serde_json::Value) {
    println!("Provider:       {provider}");
    println!(
        "Repository:     {}",
        v.get("repositoryId")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
    );
    println!(
        "Routing mode:   {}",
        v.get("routingMode")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
    );
    if let Some(pc) = v.get("providerConfig") {
        if !pc.is_null() {
            println!("Provider config: {pc}");
        }
    }
}

fn parse_provider_config(
    s: Option<&str>,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, ClientError> {
    s.map(|s| {
        let v: serde_json::Value =
            serde_json::from_str(s).map_err(|e| ClientError::Api {
                status: 0,
                body: format!("Invalid --provider-config JSON: {e}"),
            })?;
        match v {
            serde_json::Value::Object(map) => Ok(map),
            _ => Err(ClientError::Api {
                status: 0,
                body: "--provider-config must be a JSON object".to_string(),
            }),
        }
    })
    .transpose()
}

fn parse_channel_type(s: &str) -> ChannelType {
    match s {
        "email" => ChannelType::Email,
        "telegram" => ChannelType::Telegram,
        _ => ChannelType::Slack,
    }
}

fn parse_routing_mode(s: &str) -> RoutingMode {
    match s {
        "per_message" => RoutingMode::PerMessage,
        "thread_based" => RoutingMode::ThreadBased,
        _ => RoutingMode::Ongoing,
    }
}

fn format_channels(configs: &[ChannelConfigResponse]) {
    let rows: Vec<Vec<String>> = configs
        .iter()
        .map(|c| {
            vec![
                c.id.clone(),
                c.provider.to_string(),
                c.source_id.clone(),
                c.routing_mode.to_string(),
            ]
        })
        .collect();

    output::print_table(&["ID", "PROVIDER", "SOURCE", "MODE"], &rows);
}

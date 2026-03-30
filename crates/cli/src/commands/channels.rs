//! Channel commands: list, create, defaults.

use clap::Subcommand;
use moatless_api_schemas::channel::{
    ChannelConfigResponse, CreateChannelConfigRequest, SendChannelMessageRequest,
    TriggerChannelRequest, UpdateChannelConfigRequest,
};
use moatless_api_schemas::enums::{ChannelType, RoutingMode};

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
        /// Provider-specific config as JSON (e.g. '{"access_policy":"open","trigger":"all_messages"}')
        #[arg(long)]
        provider_config: Option<String>,
        /// System prompt template for the channel
        #[arg(long)]
        prompt_template: Option<String>,
        /// Default task name for tasks created from this channel
        #[arg(long)]
        task_name: Option<String>,
        /// Comma-separated tag IDs to auto-assign to tasks
        #[arg(long, value_delimiter = ',')]
        tag_ids: Option<Vec<String>>,
    },
    /// Update a channel config
    Update {
        /// Channel config ID
        id: String,
        /// Repository ID to route messages to
        #[arg(long)]
        repo: Option<String>,
        /// Routing mode (ongoing, per_message, thread_based)
        #[arg(long)]
        mode: Option<String>,
        /// Display name for the source
        #[arg(long)]
        source_name: Option<String>,
        /// Enable or disable the channel (true/false)
        #[arg(long)]
        active: Option<bool>,
        /// Provider-specific config as JSON
        #[arg(long)]
        provider_config: Option<String>,
        /// Prompt template
        #[arg(long)]
        prompt_template: Option<String>,
        /// Response config ID
        #[arg(long)]
        response_config_id: Option<String>,
        /// Default task name for tasks created from this channel
        #[arg(long)]
        task_name: Option<String>,
        /// Comma-separated tag IDs to auto-assign to tasks
        #[arg(long, value_delimiter = ',')]
        tag_ids: Option<Vec<String>>,
    },
    /// Send a message through the channel(s) associated with a task
    Send {
        /// Task ID (uses TASK_ID env var if omitted)
        #[arg(long)]
        task: Option<String>,
        /// Channel config ID (sends to all channels if omitted)
        #[arg(long)]
        channel: Option<String>,
        /// Message text (reads from stdin if omitted and stdin is piped)
        #[arg(long)]
        text: Option<String>,
    },
    /// Manually trigger a channel config, creating a task as if the provider fired
    Trigger {
        /// Channel config ID
        id: String,
        /// Optional message override (uses channel's configured message if omitted)
        #[arg(long)]
        message: Option<String>,
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
        /// Team ID to scope the defaults to
        #[arg(long)]
        team: Option<String>,
        /// Default task name for tasks created from this channel
        #[arg(long)]
        task_name: Option<String>,
        /// Comma-separated tag IDs to auto-assign to tasks
        #[arg(long, value_delimiter = ',')]
        tag_ids: Option<Vec<String>>,
    },
}

pub async fn run(
    client: &MoatClient,
    action: ChannelAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ChannelAction::List => {
            let configs = client.api().list_channel_configs(None, None).await?;
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
            prompt_template,
            task_name,
            tag_ids,
        } => {
            let parsed_config = parse_provider_config(provider_config.as_deref())?
                .map(serde_json::Value::Object);

            let req = CreateChannelConfigRequest {
                provider: parse_channel_type(&provider),
                source_id: source,
                repository_id: repo,
                routing_mode: parse_routing_mode(&mode),
                prompt_template,
                provider_config: parsed_config,
                response_config_id: None,
                scope: None,
                source_name: None,
                team_id: None,
                task_name,
                tag_ids,
            };
            let created = client.api().create_channel_config(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Channel config created: {}", created.id);
            }
        }
        ChannelAction::Update {
            id,
            repo,
            mode,
            source_name,
            active,
            provider_config,
            prompt_template,
            response_config_id,
            task_name,
            tag_ids,
        } => {
            let parsed_config = parse_provider_config(provider_config.as_deref())?
                .map(serde_json::Value::Object);

            let req = UpdateChannelConfigRequest {
                repository_id: repo,
                routing_mode: mode.as_deref().map(parse_routing_mode),
                source_name,
                active,
                provider_config: parsed_config,
                prompt_template: prompt_template.map(Some),
                response_config_id: response_config_id.map(Some),
                task_name: task_name.map(Some),
                tag_ids,
            };
            let updated = client.api().update_channel_config(&id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Channel config updated: {}", updated.id);
                format_channels(&[updated]);
            }
        }
        ChannelAction::Send {
            task,
            channel,
            text,
        } => {
            let task_id = task
                .or_else(|| std::env::var("TASK_ID").ok())
                .ok_or_else(|| {
                    ClientError::Other(
                        "Task ID required: use --task or set TASK_ID env var".into(),
                    )
                })?;

            let message_text = if let Some(t) = text {
                t
            } else {
                use std::io::{IsTerminal, Read};
                if std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Provide --text or pipe message via stdin".into(),
                    ));
                }
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| ClientError::Other(format!("Failed to read stdin: {e}")))?;
                buf.trim().to_string()
            };

            let req = SendChannelMessageRequest {
                text: message_text,
            };

            let resp = if let Some(config_id) = channel {
                client
                    .api()
                    .send_to_task_channel(&task_id, &config_id, &req)
                    .await?
            } else {
                client.api().send_to_task_channels(&task_id, &req).await?
            };

            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Sent to {} channel(s).", resp.sent_count);
                for err in &resp.errors {
                    eprintln!("Warning: {err}");
                }
            }
        }
        ChannelAction::Trigger { id, message } => {
            let req = TriggerChannelRequest { message };
            let resp = client.api().trigger_channel_config(&id, &req).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Task created: {} (short: {})", resp.task_id, resp.short_id);
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
            team,
            task_name,
            tag_ids,
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
                "teamId": team,
                "taskName": task_name,
                "tagIds": tag_ids,
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
        "github_pr" => ChannelType::GithubPr,
        "webhook" => ChannelType::Webhook,
        "cron" => ChannelType::Cron,
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

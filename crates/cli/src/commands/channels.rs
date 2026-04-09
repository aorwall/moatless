//! Channel commands: list, create, defaults.

use clap::Subcommand;
use moatless_api_schemas::channel::{
    ChannelConfigResponse, ConnectChannelRequest, CreateChannelConfigRequest, DirectSendRequest,
    SendChannelMessageRequest, TriggerChannelRequest, UpdateChannelConfigRequest,
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
        /// User ID to run tasks on behalf of (defaults to channel creator)
        #[arg(long)]
        run_as_user_id: Option<String>,
        /// Allow direct sending to this channel without a task
        #[arg(long)]
        allow_direct_send: bool,
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
        /// User ID to run tasks on behalf of (defaults to channel creator)
        #[arg(long)]
        run_as_user_id: Option<String>,
        /// Allow direct sending to this channel without a task (true/false)
        #[arg(long)]
        allow_direct_send: Option<bool>,
    },
    /// Send a message through the channel(s) associated with a task
    Send {
        /// Task ID (uses TASK_ID env var if omitted). Required unless --direct is set.
        #[arg(long)]
        task: Option<String>,
        /// Channel config ID (sends to all task channels if omitted)
        #[arg(long)]
        channel: Option<String>,
        /// Message text (reads from stdin if omitted and stdin is piped)
        #[arg(long)]
        text: Option<String>,
        /// File(s) to attach to the message
        #[arg(long = "file")]
        files: Vec<String>,
        /// Send directly to a channel config without a task (fire-and-forget)
        #[arg(long)]
        direct: bool,
    },
    /// Manually trigger a channel config, creating a task as if the provider fired
    Trigger {
        /// Channel config ID
        id: String,
        /// Optional message override (uses channel's configured message if omitted)
        #[arg(long)]
        message: Option<String>,
        /// Thread properties as JSON (required for github_pr: must include owner, repo, pr_number)
        #[arg(long)]
        thread_properties: Option<String>,
    },
    /// Connect a task to an existing external thread (e.g. a GitHub PR)
    Connect {
        /// PR number to connect to
        #[arg(long)]
        pr: u64,
        /// Repository in owner/repo format (resolved from task if omitted)
        #[arg(long)]
        repo: Option<String>,
        /// Task ID (uses TASK_ID env var if omitted)
        #[arg(long)]
        task: Option<String>,
        /// System user ID to filter out (prevents echoing agent's own comments)
        #[arg(long)]
        filter_user: Option<String>,
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
            run_as_user_id,
            allow_direct_send,
        } => {
            let parsed_config =
                parse_provider_config(provider_config.as_deref())?.map(serde_json::Value::Object);

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
                run_as_user_id,
                allow_direct_send: if allow_direct_send { Some(true) } else { None },
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
            run_as_user_id,
            allow_direct_send,
        } => {
            let parsed_config =
                parse_provider_config(provider_config.as_deref())?.map(serde_json::Value::Object);

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
                run_as_user_id: run_as_user_id.map(Some),
                allow_direct_send,
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
            files,
            direct,
        } => {
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

            // Upload files, collect file_ids
            let mut file_ids = Vec::new();
            for path_str in &files {
                let path = std::path::Path::new(path_str);
                if !path.exists() {
                    return Err(ClientError::Other(format!("File not found: {path_str}")));
                }
                let data = std::fs::read(path)
                    .map_err(|e| ClientError::Other(format!("Failed to read {path_str}: {e}")))?;
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unnamed");
                let mime_type = mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string();

                let upload_resp = client
                    .api()
                    .upload_file(filename, &mime_type, data)
                    .await
                    .map_err(|e| ClientError::Other(format!("Failed to upload {path_str}: {e}")))?;
                file_ids.push(upload_resp.file_id);
            }

            if direct {
                let config_id = channel.ok_or_else(|| {
                    ClientError::Other("--channel is required for --direct send".into())
                })?;
                // Use task_id if provided (or from env) so the thread is tracked.
                let task_id = task.or_else(|| std::env::var("TASK_ID").ok());
                let req = DirectSendRequest {
                    text: message_text,
                    file_ids,
                    task_id,
                };
                let resp = client
                    .api()
                    .direct_send_to_channel(&config_id, &req)
                    .await?;
                if json {
                    output::print_json_typed(&resp);
                } else {
                    println!("Message sent directly to channel config {config_id}.");
                    if let Some(ext_id) = &resp.external_message_id {
                        println!("External message ID: {ext_id}");
                    }
                }
            } else {
                let task_id = task
                    .or_else(|| std::env::var("TASK_ID").ok())
                    .ok_or_else(|| {
                        ClientError::Other(
                            "Task ID required: use --task or set TASK_ID env var".into(),
                        )
                    })?;

                let req = SendChannelMessageRequest {
                    text: message_text,
                    file_ids,
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
        }
        ChannelAction::Trigger {
            id,
            message,
            thread_properties,
        } => {
            let thread_properties = thread_properties
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()
                .map_err(|e| {
                    ClientError::Other(format!("Invalid JSON for --thread-properties: {e}"))
                })?;
            let req = TriggerChannelRequest {
                message,
                thread_properties,
            };
            let resp = client.api().trigger_channel_config(&id, &req).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Task created: {} (short: {})", resp.task_id, resp.short_id);
            }
        }
        ChannelAction::Connect {
            pr,
            repo,
            task,
            filter_user,
        } => {
            let task_id = task
                .or_else(|| std::env::var("TASK_ID").ok())
                .ok_or_else(|| {
                    ClientError::Other("Task ID required: use --task or set TASK_ID env var".into())
                })?;

            // Build thread_properties from owner/repo if provided
            let (source_id, thread_properties) = if let Some(ref repo_str) = repo {
                let parts: Vec<&str> = repo_str.splitn(2, '/').collect();
                if parts.len() != 2 {
                    return Err(ClientError::Other(
                        "--repo must be in owner/repo format".into(),
                    ));
                }
                let (owner, repo_name) = (parts[0], parts[1]);
                (
                    Some(repo_str.clone()),
                    Some(serde_json::json!({
                        "owner": owner,
                        "repo": repo_name,
                        "pr_number": pr,
                    })),
                )
            } else {
                // Server resolves source_id from task's repository
                (None, None)
            };

            let req = ConnectChannelRequest {
                task_id: task_id.clone(),
                provider: ChannelType::GithubPr,
                source_id,
                external_thread_id: pr.to_string(),
                thread_properties,
                filter_user_id: filter_user,
            };

            let resp = client.api().connect_channel(&req).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!(
                    "Connected task {} to PR #{} (config: {}, thread: {})",
                    task_id, pr, resp.config_id, resp.thread_id
                );
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
        v.get("routingMode").and_then(|v| v.as_str()).unwrap_or("-")
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
        let v: serde_json::Value = serde_json::from_str(s).map_err(|e| ClientError::Api {
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
                if c.allow_direct_send {
                    "yes".to_string()
                } else {
                    "-".to_string()
                },
            ]
        })
        .collect();

    output::print_table(&["ID", "PROVIDER", "SOURCE", "MODE", "SENDABLE"], &rows);
}

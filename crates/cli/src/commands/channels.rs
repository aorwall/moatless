//! Channel commands: list, create, defaults.

use clap::Subcommand;
use moatless_api_schemas::channel::{
    ChannelConfigResponse, ChatDestinationSendRequest, ConnectChannelRequest,
    CreateChannelConfigRequest, CreateIngressRuleRequest, DirectSendRequest, IngressRuleResponse,
    SendChannelMessageRequest, TriggerChannelRequest, UpdateChannelConfigRequest,
};
use moatless_api_schemas::enums::{ChannelType, RoutingMode};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum ChannelAction {
    /// List channel configs
    List,
    /// List channel subscriptions connected to a task
    Subscriptions {
        /// Task ID (uses TASK_ID env var if omitted)
        #[arg(long)]
        task: Option<String>,
    },
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
        /// Agent type for tasks created from this channel (default: claude-code)
        #[arg(long)]
        agent_type: Option<String>,
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
        /// Agent type for tasks created from this channel
        #[arg(long)]
        agent_type: Option<String>,
    },
    /// Send a message through the channel(s) associated with a task
    Send {
        /// Task ID (uses TASK_ID env var if omitted). Required unless --direct is set.
        #[arg(long)]
        task: Option<String>,
        /// Channel config ID (sends to all task channels if omitted)
        #[arg(long)]
        channel: Option<String>,
        /// Exact channel thread ID (preferred when a task has multiple subscriptions)
        #[arg(long)]
        thread: Option<i32>,
        /// Exact task subscription ID, e.g. `ts_abc...`. `--ts` is a short alias.
        #[arg(long, visible_alias = "ts", value_name = "ID")]
        subscription: Option<String>,
        /// Exact GitHub PR subscription to target, in owner/repo#123 format
        #[arg(long)]
        pr: Option<String>,
        /// Message text (reads from stdin if omitted and stdin is piped)
        #[arg(long)]
        text: Option<String>,
        /// File(s) to attach to the message
        #[arg(long = "file")]
        files: Vec<String>,
        /// Send directly to a channel config without a task (fire-and-forget)
        #[arg(long)]
        direct: bool,
        /// Provider name for raw destination send (e.g. "slack").
        /// Requires --to. When set, posts to the destination via
        /// POST /api/v1/channels/{provider}/send.
        #[arg(long)]
        provider: Option<String>,
        /// Provider-specific destination identifier (e.g. Slack channel id
        /// "C0000000005"). Requires --provider.
        #[arg(long)]
        to: Option<String>,
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
    /// Subscribe a task to a GitHub PR
    #[command(visible_alias = "subscribe")]
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
    /// Manage GitHub-native ingress rules (replaces `channels create --provider github`)
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // reason: clap expands subcommand variants from large structs; boxing complicates derive
pub enum RulesAction {
    /// List ingress rules
    List {
        /// Filter by provider (defaults to "github")
        #[arg(long)]
        provider: Option<String>,
    },
    /// Create or upsert an ingress rule for a GitHub repo
    Create {
        /// Provider (only "github" is supported today)
        #[arg(long, default_value = "github")]
        provider: String,
        /// Repo full name in `owner/repo` form (or `owner/*` wildcard)
        #[arg(long)]
        repo: String,
        /// Repository ID in moatless to route messages to
        #[arg(long = "repository-id")]
        repository_id: String,
        /// Default task name for tasks created from this rule
        #[arg(long = "task-name")]
        task_name: Option<String>,
        /// Comma-separated tag IDs to auto-assign to tasks
        #[arg(long = "tag-ids", value_delimiter = ',')]
        tag_ids: Option<Vec<String>>,
        /// System prompt template appended to channel-originated tasks
        #[arg(long = "prompt-template")]
        prompt_template: Option<String>,
        /// Agent type (default: claude-code)
        #[arg(long = "agent-type")]
        agent_type: Option<String>,
        /// User to run tasks as (falls back to created_by)
        #[arg(long = "run-as-user")]
        run_as_user_id: Option<String>,
        /// Auto-close the task when the PR is closed (default: true)
        #[arg(long = "close-task-on-close")]
        close_task_on_close: Option<bool>,
        /// Notify when a check_suite completes successfully
        #[arg(long = "notify-on-success")]
        notify_on_success: bool,
        /// Comma-separated event filter (empty means all)
        #[arg(long = "event-filter", value_delimiter = ',')]
        event_filter: Option<Vec<String>>,
        /// Team ID to scope the rule to (auto-resolved when single-team)
        #[arg(long)]
        team: Option<String>,
        /// Disable the rule on creation (default: enabled)
        #[arg(long)]
        inactive: bool,
    },
    /// Delete an ingress rule by id
    Delete {
        /// Rule id (e.g. `ir_...`)
        id: String,
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
        /// Agent type for tasks (default: claude-code)
        #[arg(long)]
        agent_type: Option<String>,
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
            let mut rules = Vec::new();
            for provider in ["github", "slack", "telegram"] {
                if let Ok(mut provider_rules) =
                    client.api().list_ingress_rules(Some(provider)).await
                {
                    rules.append(&mut provider_rules);
                }
            }
            if json {
                output::print_json_typed(&serde_json::json!({
                    "configs": configs,
                    "rules": rules,
                }));
            } else {
                format_channels(&configs);
                format_ingress_rules(&rules);
            }
        }
        ChannelAction::Subscriptions { task } => {
            let task_id = task
                .or_else(|| std::env::var("TASK_ID").ok())
                .ok_or_else(|| {
                    ClientError::Other("Task ID required: use --task or set TASK_ID env var".into())
                })?;
            let channels = client.api().list_task_channels(&task_id).await?;
            if json {
                output::print_json_typed(&channels);
            } else {
                format_task_channels(&task_id, &channels);
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
            agent_type,
        } => {
            let parsed_config =
                parse_provider_config(provider_config.as_deref())?.map(serde_json::Value::Object);

            if matches!(provider.as_str(), "github" | "github_pr" | "slack" | "telegram") {
                let req = CreateIngressRuleRequest {
                    provider: if provider == "github_pr" {
                        "github".to_string()
                    } else {
                        provider.clone()
                    },
                    scope_matcher: source,
                    source_name: None,
                    repository_id: repo,
                    routing_mode: Some(parse_routing_mode(&mode)),
                    task_name,
                    tag_ids,
                    prompt_template,
                    provider_config: parsed_config,
                    allow_direct_send: if allow_direct_send { Some(true) } else { None },
                    response_rule_id: None,
                    ongoing_task_id: None,
                    agent_type,
                    run_as_user_id,
                    close_task_on_close: None,
                    notify_on_success: None,
                    active: Some(true),
                    event_filter: None,
                    team_id: None,
                };
                let created = client.api().create_ingress_rule(&req).await?;
                if json {
                    output::print_json_typed(&created);
                } else {
                    println!("Ingress rule upserted: {}", created.id);
                }
                return Ok(());
            }

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
                agent_type,
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
            agent_type,
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
                agent_type,
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
            thread,
            subscription,
            pr,
            text,
            files,
            direct,
            provider,
            to,
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

            if provider.is_some() || to.is_some() {
                let provider = provider.ok_or_else(|| {
                    ClientError::Other("--provider is required when --to is set".into())
                })?;
                let destination = to.ok_or_else(|| {
                    ClientError::Other("--to is required when --provider is set".into())
                })?;
                if direct
                    || channel.is_some()
                    || thread.is_some()
                    || subscription.is_some()
                    || pr.is_some()
                {
                    return Err(ClientError::Other(
                        "--provider/--to is mutually exclusive with --direct, --channel, \
                         --thread, --subscription, and --pr"
                            .into(),
                    ));
                }
                let task_id = task.or_else(|| std::env::var("TASK_ID").ok());
                let req = ChatDestinationSendRequest {
                    destination: destination.clone(),
                    text: message_text,
                    task_id,
                    file_ids,
                };
                let resp = client
                    .api()
                    .send_to_chat_destination(&provider, &req)
                    .await?;
                if json {
                    output::print_json_typed(&resp);
                } else {
                    println!(
                        "Posted to {provider} destination {destination} (external id: {})",
                        resp.message_external_id,
                    );
                    if let Some(sub_id) = resp.subscription_id.as_deref() {
                        println!("Subscription: {sub_id}");
                    }
                }
                return Ok(());
            }

            if direct {
                if thread.is_some() || pr.is_some() || subscription.is_some() {
                    return Err(ClientError::Other(
                        "--thread / --subscription / --pr cannot be used with --direct send".into(),
                    ));
                }
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

                let resp = send_to_task_channel_target(
                    client,
                    &task_id,
                    channel.as_deref(),
                    thread,
                    subscription.as_deref(),
                    pr.as_deref(),
                    &req,
                )
                .await?;

                if json {
                    output::print_json_typed(&resp);
                } else {
                    if resp.targets.len() == 1 {
                        let target = &resp.targets[0];
                        println!("Sent to {}.", target.label);
                        println!("To reply in this destination, run:");
                        println!(
                            "  moat channels send {} --text \"...\"",
                            target.flag_hint
                        );
                    } else {
                        println!("Sent to {} channel(s).", resp.sent_count);
                    }
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
            } else if let Some(repo) = repo.as_deref() {
                println!(
                    "Subscribed task {} to GitHub PR {}#{} (config: {}, thread: {})",
                    task_id, repo, pr, resp.config_id, resp.thread_id
                );
            } else {
                println!(
                    "Subscribed task {} to GitHub PR #{} (config: {}, thread: {})",
                    task_id, pr, resp.config_id, resp.thread_id
                );
            }
        }
        ChannelAction::Defaults { action: defaults } => {
            run_defaults(client, defaults, json).await?;
        }
        ChannelAction::Rules { action: rules } => {
            run_rules(client, rules, json).await?;
        }
    }
    Ok(())
}

async fn run_rules(
    client: &MoatClient,
    action: RulesAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        RulesAction::List { provider } => {
            let rules = client
                .api()
                .list_ingress_rules(provider.as_deref())
                .await?;
            if json {
                output::print_json_typed(&rules);
            } else {
                format_ingress_rules(&rules);
            }
        }
        RulesAction::Create {
            provider,
            repo,
            repository_id,
            task_name,
            tag_ids,
            prompt_template,
            agent_type,
            run_as_user_id,
            close_task_on_close,
            notify_on_success,
            event_filter,
            team,
            inactive,
        } => {
            let req = CreateIngressRuleRequest {
                provider,
                scope_matcher: repo,
                source_name: None,
                repository_id,
                routing_mode: None,
                task_name,
                tag_ids,
                prompt_template,
                provider_config: None,
                allow_direct_send: None,
                response_rule_id: None,
                ongoing_task_id: None,
                agent_type,
                run_as_user_id,
                close_task_on_close,
                notify_on_success: Some(notify_on_success),
                active: Some(!inactive),
                event_filter,
                team_id: team,
            };
            let created = client.api().create_ingress_rule(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!(
                    "Ingress rule upserted: {} (scope_matcher={}, active={})",
                    created.id, created.scope_matcher, created.active
                );
            }
        }
        RulesAction::Delete { id } => {
            client.api().delete_ingress_rule(&id).await?;
            println!("Ingress rule deleted: {id}");
        }
    }
    Ok(())
}

fn format_ingress_rules(rules: &[IngressRuleResponse]) {
    if rules.is_empty() {
        println!("No ingress rules.");
        return;
    }
    println!(
        "{:<28} {:<28} {:<24} {:<7}",
        "ID", "SCOPE_MATCHER", "REPOSITORY_ID", "ACTIVE"
    );
    for rule in rules {
        println!(
            "{:<28} {:<28} {:<24} {:<7}",
            truncate_label(&rule.id, 28),
            truncate_label(&rule.scope_matcher, 28),
            truncate_label(&rule.repository_id, 24),
            rule.active
        );
    }
}

fn truncate_label(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
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
            agent_type,
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
                "agentType": agent_type,
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

pub async fn send_to_task_channel_target(
    client: &MoatClient,
    task_id: &str,
    channel: Option<&str>,
    thread: Option<i32>,
    subscription: Option<&str>,
    pr: Option<&str>,
    req: &SendChannelMessageRequest,
) -> Result<moatless_api_schemas::channel::SendChannelMessageResponse, ClientError> {
    let target_count = usize::from(channel.is_some())
        + usize::from(thread.is_some())
        + usize::from(subscription.is_some())
        + usize::from(pr.is_some());
    if target_count > 1 {
        return Err(ClientError::Other(
            "Use only one of --channel, --thread, --subscription, or --pr".into(),
        ));
    }

    if let Some(thread_id) = thread {
        return client
            .api()
            .send_to_task_channel_thread(task_id, thread_id, req)
            .await
            .map_err(ClientError::from);
    }

    if let Some(sub_id) = subscription {
        return client
            .api()
            .send_to_task_channel_subscription(task_id, sub_id, req)
            .await
            .map_err(ClientError::from);
    }

    if let Some(subject_key) = pr {
        let channels = client.api().list_task_channels(task_id).await?;
        let matches: Vec<_> = channels
            .into_iter()
            .filter(|channel| {
                channel.provider == ChannelType::GithubPr
                    && channel.subject_key.as_deref() == Some(subject_key)
            })
            .collect();

        let channel = match matches.as_slice() {
            [] => {
                return Err(ClientError::Other(format!(
                    "Task {task_id} is not subscribed to GitHub PR {subject_key}"
                )));
            }
            [channel] => channel,
            _ => {
                return Err(ClientError::Other(format!(
                    "Task {task_id} has multiple subscriptions for {subject_key}; \
                     use --subscription <id>"
                )));
            }
        };

        if let Some(sub_id) = channel.subscription_id.as_deref() {
            return client
                .api()
                .send_to_task_channel_subscription(task_id, sub_id, req)
                .await
                .map_err(ClientError::from);
        }

        if let Some(thread_id) = channel.thread_id {
            return client
                .api()
                .send_to_task_channel_thread(task_id, thread_id, req)
                .await
                .map_err(ClientError::from);
        }

        return Err(ClientError::Other(format!(
            "GitHub PR subscription {subject_key} has no addressable id; \
             list channels via `moat channels subscriptions --task {task_id}`"
        )));
    }

    if let Some(config_id) = channel {
        return client
            .api()
            .send_to_task_channel(task_id, config_id, req)
            .await
            .map_err(ClientError::from);
    }

    match client.api().send_to_task_channels(task_id, req).await {
        Ok(resp) => Ok(resp),
        Err(err) => Err(map_send_to_channels_error(err)),
    }
}

/// Translate a `POST /channels/send` failure into `ClientError`. When the
/// backend returns the structured `MultipleTargets` body, render it as a
/// multi-line message so the agent sees exactly which flags to use.
fn map_send_to_channels_error(err: moatless_api_client::ApiError) -> ClientError {
    if let moatless_api_client::ApiError::Api { status: 400, body } = &err {
        if let Ok(parsed) =
            serde_json::from_str::<moatless_api_schemas::channel::MultipleTargetsErrorBody>(body)
        {
            let mut lines = vec![
                parsed.error,
                parsed.hint,
                String::new(),
                "Targets:".to_string(),
            ];
            for target in &parsed.targets {
                lines.push(format!("  - {} ({})", target.label, target.flag_hint));
            }
            return ClientError::Other(lines.join("\n"));
        }
    }
    ClientError::from(err)
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
        "github" => ChannelType::GithubPr,
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
                display_channel_type(c.provider),
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
    if configs
        .iter()
        .any(|config| config.provider == ChannelType::GithubPr)
    {
        println!();
        println!(
            "GitHub channel configs are ingress configs. Use `moat channels subscriptions --task <task>` to inspect PR subscriptions."
        );
    }
}

fn format_task_channels(
    task_id: &str,
    channels: &[moatless_api_schemas::channel::TaskChannelResponse],
) {
    if channels.is_empty() {
        println!("No channel subscriptions for task {task_id}.");
        return;
    }

    let rows: Vec<Vec<String>> = channels
        .iter()
        .map(|channel| {
            vec![
                display_channel_type(channel.provider),
                channel.config_id.clone(),
                channel
                    .thread_id
                    .map_or_else(|| "-".to_string(), |id| id.to_string()),
                channel
                    .subject_key
                    .clone()
                    .or_else(|| channel.external_thread_id.clone())
                    .unwrap_or_else(|| "-".to_string()),
                channel.source_id.clone(),
                channel
                    .created_reason
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
                channel
                    .last_event_at
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();

    output::print_table(
        &[
            "PROVIDER",
            "CONFIG",
            "THREAD",
            "SUBJECT",
            "SOURCE",
            "REASON",
            "LAST_EVENT",
        ],
        &rows,
    );
}

fn display_channel_type(provider: ChannelType) -> String {
    match provider {
        ChannelType::GithubPr => "github".to_string(),
        _ => provider.to_string(),
    }
}

//! Canonical integration commands.

use clap::Subcommand;
use moatless_api_schemas::channel::{
    CreateProviderConnectionRequest, CreateScheduledJobRequest, CreateTaskBindingRequest,
    CreateTaskRouteRequest, CreateTaskTriggerRequest,
};
use moatless_api_schemas::enums::{IntegrationProvider, RoutingMode};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum IntegrationsAction {
    /// Manage provider connections
    Connections {
        #[command(subcommand)]
        action: ConnectionAction,
    },
}

#[derive(Subcommand)]
pub enum ConnectionAction {
    /// List provider connections
    List {
        #[arg(long)]
        provider: Option<String>,
    },
    /// Create or update a provider connection
    Create {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        external_account_id: String,
        #[arg(long)]
        connection_kind: Option<String>,
        #[arg(long)]
        webhook_secret: Option<String>,
        #[arg(long)]
        transport_config: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TaskRouteAction {
    /// List task routes
    List {
        #[arg(long)]
        repository_id: Option<String>,
    },
    /// Create a task route
    Create {
        #[arg(long)]
        repository_id: String,
        #[arg(long, default_value = "thread_based")]
        routing_mode: String,
        #[arg(long)]
        prompt_template: Option<String>,
        #[arg(long)]
        task_name: Option<String>,
        #[arg(long)]
        run_as_user_id: Option<String>,
        #[arg(long, default_value = "claude-code")]
        agent_type: String,
        #[arg(long = "tag")]
        tag_ids: Vec<String>,
        #[arg(long)]
        team_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TriggerAction {
    /// List triggers
    List {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        route_id: Option<String>,
    },
    /// Create a trigger
    Create {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        connection_id: String,
        #[arg(long)]
        route_id: String,
        #[arg(long)]
        source_matcher: String,
        #[arg(long)]
        source_name: Option<String>,
        #[arg(long)]
        event_filter: Option<String>,
        #[arg(long)]
        trigger_config: Option<String>,
        #[arg(long, default_value_t = false)]
        allow_direct_send: bool,
        #[arg(long)]
        team_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ScheduledJobAction {
    /// List scheduled jobs
    List,
    /// Create a scheduled job
    Create {
        #[arg(long)]
        route_id: String,
        #[arg(long)]
        schedule_expr: String,
        #[arg(long)]
        message_template: String,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        run_as_user_id: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum BindingAction {
    /// List task bindings
    List {
        #[arg(long)]
        task: String,
    },
    /// Create a task binding
    Create {
        #[arg(long)]
        task: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        connection_id: String,
        #[arg(long)]
        subject_kind: String,
        #[arg(long)]
        subject_key: String,
        #[arg(long)]
        trigger_id: Option<String>,
        #[arg(long)]
        destination_config: Option<String>,
        #[arg(long)]
        metadata: Option<String>,
    },
}

pub async fn run_integrations(
    client: &MoatClient,
    action: IntegrationsAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        IntegrationsAction::Connections { action } => run_connections(client, action, json).await,
    }
}

async fn run_connections(
    client: &MoatClient,
    action: ConnectionAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ConnectionAction::List { provider } => {
            let items = client
                .api()
                .list_integration_connections(provider.as_deref())
                .await?;
            if json {
                output::print_json_typed(&items);
            } else {
                for item in items {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.id, item.provider, item.connection_kind, item.external_account_id
                    );
                }
            }
        }
        ConnectionAction::Create {
            provider,
            external_account_id,
            connection_kind,
            webhook_secret,
            transport_config,
            team_id,
        } => {
            let req = CreateProviderConnectionRequest {
                provider: parse_provider(&provider)?,
                connection_kind,
                external_account_id,
                webhook_secret,
                source_name: None,
                transport_config: parse_json_opt(transport_config.as_deref())?,
                team_id,
            };
            let created = client.api().create_integration_connection(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Connection upserted: {}", created.id);
            }
        }
    }
    Ok(())
}

pub async fn run_task_routes(
    client: &MoatClient,
    action: TaskRouteAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        TaskRouteAction::List { repository_id } => {
            let items = client
                .api()
                .list_task_routes(repository_id.as_deref())
                .await?;
            print_or_json(&items, json);
        }
        TaskRouteAction::Create {
            repository_id,
            routing_mode,
            prompt_template,
            task_name,
            run_as_user_id,
            agent_type,
            tag_ids,
            team_id,
        } => {
            let req = CreateTaskRouteRequest {
                repository_id,
                routing_mode: parse_routing_mode(&routing_mode)?,
                ongoing_task_id: None,
                prompt_template,
                task_name,
                tag_ids: Some(tag_ids),
                run_as_user_id,
                agent_type: Some(agent_type),
                scope: None,
                team_id,
            };
            let created = client.api().create_task_route(&req).await?;
            print_or_json(&created, json);
        }
    }
    Ok(())
}

pub async fn run_triggers(
    client: &MoatClient,
    action: TriggerAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        TriggerAction::List { provider, route_id } => {
            let items = client
                .api()
                .list_task_triggers(provider.as_deref(), route_id.as_deref())
                .await?;
            print_or_json(&items, json);
        }
        TriggerAction::Create {
            provider,
            connection_id,
            route_id,
            source_matcher,
            source_name,
            event_filter,
            trigger_config,
            allow_direct_send,
            team_id,
        } => {
            let req = CreateTaskTriggerRequest {
                provider: parse_provider(&provider)?,
                connection_id,
                source_matcher,
                source_name,
                event_filter: parse_json_opt(event_filter.as_deref())?,
                trigger_config: parse_json_opt(trigger_config.as_deref())?,
                route_id,
                allow_direct_send: Some(allow_direct_send),
                close_task_on_close: None,
                notify_on_success: None,
                active: None,
                scope: None,
                team_id,
            };
            let created = client.api().create_task_trigger(&req).await?;
            print_or_json(&created, json);
        }
    }
    Ok(())
}

pub async fn run_scheduled_jobs(
    client: &MoatClient,
    action: ScheduledJobAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ScheduledJobAction::List => {
            let items = client.api().list_scheduled_jobs().await?;
            print_or_json(&items, json);
        }
        ScheduledJobAction::Create {
            route_id,
            schedule_expr,
            message_template,
            timezone,
            run_as_user_id,
            team_id,
        } => {
            let req = CreateScheduledJobRequest {
                route_id,
                schedule_expr,
                timezone,
                message_template,
                run_as_user_id,
                active: None,
                scope: None,
                team_id,
            };
            let created = client.api().create_scheduled_job(&req).await?;
            print_or_json(&created, json);
        }
    }
    Ok(())
}

pub async fn run_bindings(
    client: &MoatClient,
    action: BindingAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        BindingAction::List { task } => {
            let items = client.api().list_task_bindings(&task).await?;
            print_or_json(&items, json);
        }
        BindingAction::Create {
            task,
            provider,
            connection_id,
            subject_kind,
            subject_key,
            trigger_id,
            destination_config,
            metadata,
        } => {
            let req = CreateTaskBindingRequest {
                provider: parse_provider(&provider)?,
                connection_id,
                trigger_id,
                subject_kind,
                subject_key,
                destination_config: parse_json_opt(destination_config.as_deref())?,
                metadata: parse_json_opt(metadata.as_deref())?,
                origin: Some("api".to_string()),
            };
            let created = client.api().create_task_binding(&task, &req).await?;
            print_or_json(&created, json);
        }
    }
    Ok(())
}

fn parse_provider(value: &str) -> Result<IntegrationProvider, ClientError> {
    IntegrationProvider::parse(value).ok_or_else(|| {
        ClientError::Other(format!(
            "Invalid provider '{value}'. Expected slack, telegram, github, linear, or webhook"
        ))
    })
}

fn parse_routing_mode(value: &str) -> Result<RoutingMode, ClientError> {
    RoutingMode::try_from(value).map_err(ClientError::Other)
}

fn parse_json_opt(value: Option<&str>) -> Result<Option<serde_json::Value>, ClientError> {
    value
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|e| ClientError::Other(format!("Invalid JSON '{raw}': {e}")))
        })
        .transpose()
}

fn print_or_json<T>(value: &T, json: bool)
where
    T: serde::Serialize + std::fmt::Debug,
{
    if json {
        output::print_json_typed(value);
    } else {
        println!("{value:#?}");
    }
}

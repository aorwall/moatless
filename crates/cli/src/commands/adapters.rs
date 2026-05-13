//! Adapter, trigger, and binding commands.

use std::io::IsTerminal;

use clap::Subcommand;
use moatless_api_schemas::adapters::CreateAdapterConnectionRequest;
use moatless_api_schemas::bindings::CreateTaskBindingRequest;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode, Scope};
use moatless_api_schemas::triggers::{
    CreateAdapterEventTriggerSource, CreateScheduleTriggerSource, CreateTriggerRequest,
    CreateTriggerRule, CreateTriggerSource, TriggerKind, UpdateAdapterEventTriggerSource,
    UpdateScheduleTriggerSource, UpdateTriggerRequest, UpdateTriggerRule, UpdateTriggerSource,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum AdaptersAction {
    /// Manage adapter connections
    Connections {
        #[command(subcommand)]
        action: ConnectionAction,
    },
    /// List supported adapters configured on the backend
    List,
    /// Launch the OAuth install flow for an adapter.
    ///
    /// Builds the backend install URL and opens it in your browser
    /// (use `--print` to copy/paste instead). Currently supports `linear`.
    Install {
        #[arg(long)]
        adapter: String,
        /// Print the install URL instead of trying to open it in a browser.
        #[arg(long, default_value_t = false)]
        print: bool,
    },
}

#[derive(Subcommand)]
pub enum ConnectionAction {
    /// List adapter connections
    List {
        #[arg(long)]
        adapter: Option<String>,
    },
    /// Create or update an adapter connection
    Create {
        #[arg(long)]
        adapter: String,
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
pub enum TriggerAction {
    /// List triggers
    List {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        adapter: Option<String>,
    },
    /// Create an adapter-event trigger
    CreateAdapterEvent {
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        connection_id: String,
        #[arg(long)]
        source_matcher: String,
        #[arg(long)]
        source_name: Option<String>,
        #[arg(long)]
        event_filter: Option<String>,
        #[arg(long)]
        adapter_config: Option<String>,
        #[arg(long, default_value_t = false)]
        allow_direct_send: bool,
        #[arg(long, default_value_t = true)]
        close_task_on_close: bool,
        #[arg(long, default_value_t = false)]
        notify_on_success: bool,
        #[command(flatten)]
        rule: CreateRuleArgs,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
    },
    /// Create a schedule trigger
    CreateSchedule {
        #[arg(long)]
        cron_expression: String,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        message_template: String,
        #[command(flatten)]
        rule: CreateRuleArgs,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
    },
    /// Update trigger base, rule, or source fields
    Update {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        clear_name: bool,
        #[arg(long, value_name = "BOOL")]
        active: Option<bool>,
        #[arg(long)]
        repository_id: Option<String>,
        #[arg(long)]
        routing_mode: Option<String>,
        #[arg(long)]
        ongoing_task_id: Option<String>,
        #[arg(long)]
        clear_ongoing_task_id: bool,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        clear_prompt: bool,
        #[arg(long)]
        task_name: Option<String>,
        #[arg(long)]
        clear_task_name: bool,
        #[arg(long = "tag")]
        tag_ids: Vec<String>,
        #[arg(long)]
        clear_tags: bool,
        #[arg(long)]
        run_as_user_id: Option<String>,
        #[arg(long)]
        clear_run_as_user_id: bool,
        #[arg(long)]
        agent_type: Option<String>,
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long)]
        source_matcher: Option<String>,
        #[arg(long)]
        source_name: Option<String>,
        #[arg(long)]
        clear_source_name: bool,
        #[arg(long)]
        event_filter: Option<String>,
        #[arg(long)]
        adapter_config: Option<String>,
        #[arg(long)]
        clear_adapter_config: bool,
        #[arg(long, value_name = "BOOL")]
        allow_direct_send: Option<bool>,
        #[arg(long, value_name = "BOOL")]
        close_task_on_close: Option<bool>,
        #[arg(long, value_name = "BOOL")]
        notify_on_success: Option<bool>,
        #[arg(long)]
        cron_expression: Option<String>,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        message_template: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
        #[arg(long)]
        clear_team_id: bool,
    },
    /// Delete a trigger
    Delete {
        #[arg(long)]
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args)]
pub struct CreateRuleArgs {
    #[arg(long)]
    repository_id: String,
    #[arg(long, default_value = "by_subject")]
    routing_mode: String,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    task_name: Option<String>,
    #[arg(long)]
    run_as_user_id: Option<String>,
    #[arg(long, default_value = "claude-code")]
    agent_type: String,
    #[arg(long = "tag")]
    tag_ids: Vec<String>,
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
        adapter: String,
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

pub async fn run_adapters(
    client: &MoatClient,
    action: AdaptersAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        AdaptersAction::Connections { action } => run_connections(client, action, json).await,
        AdaptersAction::List => {
            let adapters = client.api().list_adapters().await?;
            if json {
                output::print_json_typed(&adapters);
            } else {
                for adapter in adapters {
                    println!("{adapter}");
                }
            }
            Ok(())
        }
        AdaptersAction::Install { adapter, print } => run_install(client, &adapter, print),
    }
}

fn run_install(client: &MoatClient, adapter: &str, print_only: bool) -> Result<(), ClientError> {
    let kind = parse_adapter_kind(adapter)?;
    let path = match kind {
        AdapterKind::Linear => "/api/v1/auth/linear/install",
        other => {
            return Err(ClientError::Other(format!(
                "Install flow not supported for adapter '{}'",
                other.as_str()
            )));
        }
    };
    let url = format!("{}{path}", client.base_url().trim_end_matches('/'));
    if print_only {
        println!("{url}");
        return Ok(());
    }
    println!("Opening {url} in your browser…");
    if let Err(e) = open::that(&url) {
        println!(
            "Could not open browser automatically ({e}). Visit this URL manually:\n  {url}"
        );
    }
    Ok(())
}

async fn run_connections(
    client: &MoatClient,
    action: ConnectionAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ConnectionAction::List { adapter } => {
            let items = client
                .api()
                .list_adapter_connections(adapter.as_deref())
                .await?;
            if json {
                output::print_json_typed(&items);
            } else {
                for item in items {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.id, item.adapter_kind, item.connection_kind, item.external_account_id
                    );
                }
            }
        }
        ConnectionAction::Create {
            adapter,
            external_account_id,
            connection_kind,
            webhook_secret,
            transport_config,
            team_id,
        } => {
            let req = CreateAdapterConnectionRequest {
                adapter_kind: parse_adapter_kind(&adapter)?,
                connection_kind,
                external_account_id,
                webhook_secret,
                source_name: None,
                transport_config: parse_json_opt(transport_config.as_deref())?,
                team_id,
                scope: None,
            };
            let created = client.api().create_adapter_connection(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Connection upserted: {}", created.id);
            }
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
        TriggerAction::List { kind, adapter } => {
            let kind = kind.as_deref().map(parse_trigger_kind).transpose()?;
            let adapter = adapter.as_deref().map(parse_adapter_kind).transpose()?;
            let items = client
                .api()
                .list_triggers(
                    kind.as_ref().map(TriggerKind::as_str),
                    adapter.as_ref().map(AdapterKind::as_str),
                )
                .await?;
            print_or_json(&items, json);
        }
        TriggerAction::CreateAdapterEvent {
            adapter,
            connection_id,
            source_matcher,
            source_name,
            event_filter,
            adapter_config,
            allow_direct_send,
            close_task_on_close,
            notify_on_success,
            rule,
            name,
            team_id,
        } => {
            let req = CreateTriggerRequest {
                name,
                kind: TriggerKind::AdapterEvent,
                active: None,
                scope: None,
                team_id,
                rule: create_rule(rule)?,
                source: CreateTriggerSource::AdapterEvent(CreateAdapterEventTriggerSource {
                    adapter_kind: parse_adapter_kind(&adapter)?,
                    connection_id,
                    source_matcher,
                    source_name,
                    event_filter: parse_json_opt(event_filter.as_deref())?,
                    adapter_config: parse_json_opt(adapter_config.as_deref())?,
                    allow_direct_send: Some(allow_direct_send),
                    close_task_on_close: Some(close_task_on_close),
                    notify_on_success: Some(notify_on_success),
                }),
            };
            let created = client.api().create_trigger(&req).await?;
            print_or_json(&created, json);
        }
        TriggerAction::CreateSchedule {
            cron_expression,
            timezone,
            message_template,
            rule,
            name,
            team_id,
        } => {
            let req = CreateTriggerRequest {
                name,
                kind: TriggerKind::Schedule,
                active: None,
                scope: None,
                team_id,
                rule: create_rule(rule)?,
                source: CreateTriggerSource::Schedule(CreateScheduleTriggerSource {
                    cron_expression,
                    timezone,
                    message_template,
                }),
            };
            let created = client.api().create_trigger(&req).await?;
            print_or_json(&created, json);
        }
        TriggerAction::Update {
            id,
            name,
            clear_name,
            active,
            repository_id,
            routing_mode,
            ongoing_task_id,
            clear_ongoing_task_id,
            prompt,
            clear_prompt,
            task_name,
            clear_task_name,
            tag_ids,
            clear_tags,
            run_as_user_id,
            clear_run_as_user_id,
            agent_type,
            connection_id,
            source_matcher,
            source_name,
            clear_source_name,
            event_filter,
            adapter_config,
            clear_adapter_config,
            allow_direct_send,
            close_task_on_close,
            notify_on_success,
            cron_expression,
            timezone,
            message_template,
            scope,
            team_id,
            clear_team_id,
        } => {
            let routing_mode = routing_mode
                .as_deref()
                .map(parse_routing_mode)
                .transpose()?;
            let scope = scope.as_deref().map(parse_scope).transpose()?;
            let rule = update_rule(
                repository_id,
                routing_mode,
                ongoing_task_id,
                clear_ongoing_task_id,
                prompt,
                clear_prompt,
                task_name,
                clear_task_name,
                tag_ids,
                clear_tags,
                run_as_user_id,
                clear_run_as_user_id,
                agent_type,
            )?;
            let source = update_source(
                connection_id,
                source_matcher,
                source_name,
                clear_source_name,
                event_filter,
                adapter_config,
                clear_adapter_config,
                allow_direct_send,
                close_task_on_close,
                notify_on_success,
                cron_expression,
                timezone,
                message_template,
            )?;
            let req = UpdateTriggerRequest {
                name: fold_clearable(name, clear_name, "name")?,
                active,
                scope,
                team_id: fold_clearable(team_id, clear_team_id, "team-id")?,
                rule,
                source,
            };
            let updated = client.api().update_trigger(&id, &req).await?;
            print_or_json(&updated, json);
        }
        TriggerAction::Delete { id, yes } => {
            if !yes {
                confirm_delete(&id)?;
            }
            client.api().delete_trigger(&id).await?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else {
                println!("Deleted trigger {id}");
            }
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
            adapter,
            connection_id,
            subject_kind,
            subject_key,
            trigger_id,
            destination_config,
            metadata,
        } => {
            let req = CreateTaskBindingRequest {
                adapter_kind: parse_adapter_kind(&adapter)?,
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

fn create_rule(args: CreateRuleArgs) -> Result<CreateTriggerRule, ClientError> {
    Ok(CreateTriggerRule {
        repository_id: args.repository_id,
        routing_mode: parse_routing_mode(&args.routing_mode)?,
        ongoing_task_id: None,
        task_name: args.task_name,
        prompt: args.prompt,
        tag_ids: Some(args.tag_ids),
        run_as_user_id: args.run_as_user_id,
        agent_type: Some(args.agent_type),
    })
}

// reason: the trigger-rule update API exposes a clear/value pair per
// nullable field, so the bool count is intrinsic to the surface.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn update_rule(
    repository_id: Option<String>,
    routing_mode: Option<RoutingMode>,
    ongoing_task_id: Option<String>,
    clear_ongoing_task_id: bool,
    prompt: Option<String>,
    clear_prompt: bool,
    task_name: Option<String>,
    clear_task_name: bool,
    tag_ids: Vec<String>,
    clear_tags: bool,
    run_as_user_id: Option<String>,
    clear_run_as_user_id: bool,
    agent_type: Option<String>,
) -> Result<Option<UpdateTriggerRule>, ClientError> {
    let tag_ids = fold_tag_ids(tag_ids, clear_tags)?;
    let rule = UpdateTriggerRule {
        repository_id,
        routing_mode,
        ongoing_task_id: fold_clearable(ongoing_task_id, clear_ongoing_task_id, "ongoing-task-id")?,
        prompt: fold_clearable(prompt, clear_prompt, "prompt")?,
        task_name: fold_clearable(task_name, clear_task_name, "task-name")?,
        tag_ids,
        run_as_user_id: fold_clearable(run_as_user_id, clear_run_as_user_id, "run-as-user-id")?,
        agent_type,
    };
    if rule.repository_id.is_none()
        && rule.routing_mode.is_none()
        && rule.ongoing_task_id.is_none()
        && rule.prompt.is_none()
        && rule.task_name.is_none()
        && rule.tag_ids.is_none()
        && rule.run_as_user_id.is_none()
        && rule.agent_type.is_none()
    {
        Ok(None)
    } else {
        Ok(Some(rule))
    }
}

#[allow(clippy::too_many_arguments)]
fn update_source(
    connection_id: Option<String>,
    source_matcher: Option<String>,
    source_name: Option<String>,
    clear_source_name: bool,
    event_filter: Option<String>,
    adapter_config: Option<String>,
    clear_adapter_config: bool,
    allow_direct_send: Option<bool>,
    close_task_on_close: Option<bool>,
    notify_on_success: Option<bool>,
    cron_expression: Option<String>,
    timezone: Option<String>,
    message_template: Option<String>,
) -> Result<Option<UpdateTriggerSource>, ClientError> {
    let adapter_source = UpdateAdapterEventTriggerSource {
        connection_id,
        source_matcher,
        source_name: fold_clearable(source_name, clear_source_name, "source-name")?,
        event_filter: parse_json_opt(event_filter.as_deref())?,
        adapter_config: fold_clearable_json(
            adapter_config.as_deref(),
            clear_adapter_config,
            "adapter-config",
        )?,
        allow_direct_send,
        close_task_on_close,
        notify_on_success,
    };
    let has_adapter_source = adapter_source.connection_id.is_some()
        || adapter_source.source_matcher.is_some()
        || adapter_source.source_name.is_some()
        || adapter_source.event_filter.is_some()
        || adapter_source.adapter_config.is_some()
        || adapter_source.allow_direct_send.is_some()
        || adapter_source.close_task_on_close.is_some()
        || adapter_source.notify_on_success.is_some();

    let schedule_source = UpdateScheduleTriggerSource {
        cron_expression,
        timezone,
        message_template,
    };
    let has_schedule_source = schedule_source.cron_expression.is_some()
        || schedule_source.timezone.is_some()
        || schedule_source.message_template.is_some();

    match (has_adapter_source, has_schedule_source) {
        (false, false) => Ok(None),
        (true, false) => Ok(Some(UpdateTriggerSource::AdapterEvent(adapter_source))),
        (false, true) => Ok(Some(UpdateTriggerSource::Schedule(schedule_source))),
        (true, true) => Err(ClientError::Other(
            "Adapter-event and schedule source fields cannot be updated in the same command"
                .to_string(),
        )),
    }
}

fn confirm_delete(id: &str) -> Result<(), ClientError> {
    use std::io::Write;

    if !std::io::stdin().is_terminal() {
        return Err(ClientError::Other(
            "Refusing to delete without --yes in non-interactive mode".to_string(),
        ));
    }
    print!("Delete trigger {id}? This cannot be undone. [y/N] ");
    std::io::stdout()
        .flush()
        .map_err(|e| ClientError::Other(format!("Failed to flush stdout: {e}")))?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| ClientError::Other(format!("Failed to read stdin: {e}")))?;
    let answer = line.trim().to_lowercase();
    if answer != "y" && answer != "yes" {
        return Err(ClientError::Other("Aborted.".to_string()));
    }
    Ok(())
}

fn parse_adapter_kind(value: &str) -> Result<AdapterKind, ClientError> {
    AdapterKind::parse(value).ok_or_else(|| {
        ClientError::Other(format!(
            "Invalid adapter '{value}'. Expected slack, telegram, github, linear, or webhook"
        ))
    })
}

fn parse_trigger_kind(value: &str) -> Result<TriggerKind, ClientError> {
    TriggerKind::try_from(value).map_err(ClientError::Other)
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

/// Combine `--field VAL` and `--clear-field` into `Option<Option<T>>`.
#[allow(clippy::option_option)]
fn fold_clearable<T>(
    value: Option<T>,
    clear: bool,
    name: &str,
) -> Result<Option<Option<T>>, ClientError> {
    match (value, clear) {
        (Some(_), true) => Err(ClientError::Other(format!(
            "--{name} and --clear-{name} are mutually exclusive"
        ))),
        (Some(v), false) => Ok(Some(Some(v))),
        (None, true) => Ok(Some(None)),
        (None, false) => Ok(None),
    }
}

#[allow(clippy::option_option)]
fn fold_clearable_json(
    value: Option<&str>,
    clear: bool,
    name: &str,
) -> Result<Option<Option<serde_json::Value>>, ClientError> {
    let parsed = parse_json_opt(value)?;
    fold_clearable(parsed, clear, name)
}

fn fold_tag_ids(tag_ids: Vec<String>, clear: bool) -> Result<Option<Vec<String>>, ClientError> {
    match (tag_ids.is_empty(), clear) {
        (false, true) => Err(ClientError::Other(
            "--tag and --clear-tags are mutually exclusive".to_string(),
        )),
        (false, false) => Ok(Some(tag_ids)),
        (true, true) => Ok(Some(vec![])),
        (true, false) => Ok(None),
    }
}

fn parse_scope(value: &str) -> Result<Scope, ClientError> {
    match value {
        "global" => Ok(Scope::Global),
        "team" => Ok(Scope::Team),
        "user" => Ok(Scope::User),
        other => Err(ClientError::Other(format!(
            "Invalid scope '{other}'. Expected global, team, or user"
        ))),
    }
}

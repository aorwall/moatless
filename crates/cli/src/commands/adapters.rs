//! Adapter, trigger, and binding commands.

use std::io::IsTerminal;

use clap::Subcommand;
use moatless_api_schemas::adapters::CreateAdapterConnectionRequest;
use moatless_api_schemas::bindings::CreateTaskBindingRequest;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode, Scope};
use moatless_api_schemas::triggers::{
    CreateAdapterEventTriggerSource, CreateScheduleTriggerSource, CreateTriggerRequest,
    CreateTriggerRule, CreateTriggerSource, TriggerKind, TriggerSource,
    UpdateAdapterEventTriggerSource, UpdateScheduleTriggerSource, UpdateTriggerRequest,
    UpdateTriggerRule, UpdateTriggerSource,
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
    ///
    /// Adapter-specific knobs live in a free-form `adapter_config` JSON object.
    /// Set them with repeatable ops instead of hand-writing JSON:
    ///   --set KEY=VALUE     set/overwrite a field (VALUE is JSON if valid, else a string)
    ///   --append KEY=VALUE  push onto an array field (creates it if absent)
    ///   --unset KEY         remove a field
    /// or pass the whole object verbatim with --adapter-config '<json>'.
    ///
    /// Known github_pr keys: ignored_senders (string[]), ignored_title_patterns
    /// (case-insensitive title regexes, string[]), close_task_on_close, notify_on_success.
    /// The backend validates values (e.g. compiles each regex) — this command does not.
    ///
    /// Examples:
    ///   # skip dependency PRs by title
    ///   moat triggers create-adapter-event --adapter github --connection-id <c> \
    ///     --source-matcher owner/repo --repository-id <r> \
    ///     --append ignored_title_patterns='^chore\(deps' \
    ///     --append ignored_title_patterns='^deps[:(]'
    ///
    ///   # force a JSON-looking value to stay a string by quoting it
    ///   --set some_key='"123"'
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
        /// Set an adapter_config field: KEY=VALUE. VALUE is parsed as JSON when valid
        /// (`true`, `5`, `["a"]`, `{"k":1}`), otherwise treated as a string. Repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        config_set: Vec<String>,
        /// Append a value to an array field in adapter_config: KEY=VALUE. Creates the
        /// array if absent. Repeatable. e.g. --append ignored_title_patterns='^chore\(deps'
        #[arg(long = "append", value_name = "KEY=VALUE")]
        config_append: Vec<String>,
        /// Remove a field from adapter_config. Repeatable. e.g. --unset ignored_senders
        #[arg(long = "unset", value_name = "KEY")]
        config_unset: Vec<String>,
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
    ///
    /// Adapter-specific knobs live in a free-form `adapter_config` JSON object.
    /// Set them with repeatable ops instead of hand-writing JSON:
    ///   --set KEY=VALUE     set/overwrite a field (VALUE is JSON if valid, else a string)
    ///   --append KEY=VALUE  push onto an array field (creates it if absent)
    ///   --unset KEY         remove a field
    /// or pass the whole object verbatim with --adapter-config '<json>'.
    ///
    /// --set/--append/--unset fetch-merge the trigger's current adapter_config (other
    /// keys are preserved); --adapter-config replaces it wholesale; --clear-adapter-config
    /// removes it.
    ///
    /// Known github_pr keys: ignored_senders (string[]), ignored_title_patterns
    /// (case-insensitive title regexes, string[]), close_task_on_close, notify_on_success.
    /// The backend validates values (e.g. compiles each regex) — this command does not.
    ///
    /// Examples:
    ///   # append a title pattern, keeping other keys (e.g. ignored_senders)
    ///   moat triggers update --id <id> --append ignored_title_patterns='^build\(deps'
    ///
    ///   # force a JSON-looking value to stay a string by quoting it
    ///   --set some_key='"123"'
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
        #[arg(long, value_name = "DAYS")]
        inactive_ttl_days: Option<i32>,
        #[arg(long)]
        clear_inactive_ttl_days: bool,
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
        /// Set an adapter_config field: KEY=VALUE. VALUE is parsed as JSON when valid
        /// (`true`, `5`, `["a"]`, `{"k":1}`), otherwise treated as a string. Repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        config_set: Vec<String>,
        /// Append a value to an array field in adapter_config: KEY=VALUE. Creates the
        /// array if absent. Repeatable. e.g. --append ignored_title_patterns='^chore\(deps'
        #[arg(long = "append", value_name = "KEY=VALUE")]
        config_append: Vec<String>,
        /// Remove a field from adapter_config. Repeatable. e.g. --unset ignored_senders
        #[arg(long = "unset", value_name = "KEY")]
        config_unset: Vec<String>,
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
    /// Per-trigger inactivity TTL in days. Tasks spawned by this trigger
    /// auto-close after this many days of inactivity (0 = never close).
    /// Omit to inherit the platform-wide default (TASK_INACTIVE_TIMEOUT_DAYS).
    #[arg(long)]
    inactive_ttl_days: Option<i32>,
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
            config_set,
            config_append,
            config_unset,
            allow_direct_send,
            close_task_on_close,
            notify_on_success,
            rule,
            name,
            team_id,
        } => {
            let base = parse_json_opt(adapter_config.as_deref())?;
            let adapter_config = apply_config_ops(base, config_set, config_append, config_unset)?;
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
                    adapter_config,
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
            inactive_ttl_days,
            clear_inactive_ttl_days,
            connection_id,
            source_matcher,
            source_name,
            clear_source_name,
            event_filter,
            adapter_config,
            clear_adapter_config,
            config_set,
            config_append,
            config_unset,
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
            let adapter_config = resolve_update_adapter_config(
                client,
                &id,
                adapter_config,
                clear_adapter_config,
                config_set,
                config_append,
                config_unset,
            )
            .await?;
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
                inactive_ttl_days,
                clear_inactive_ttl_days,
            )?;
            let source = update_source(
                connection_id,
                source_matcher,
                source_name,
                clear_source_name,
                event_filter,
                adapter_config,
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
        inactive_ttl_days: args.inactive_ttl_days,
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
    inactive_ttl_days: Option<i32>,
    clear_inactive_ttl_days: bool,
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
        inactive_ttl_days: fold_clearable(
            inactive_ttl_days,
            clear_inactive_ttl_days,
            "inactive-ttl-days",
        )?,
    };
    if rule.repository_id.is_none()
        && rule.routing_mode.is_none()
        && rule.ongoing_task_id.is_none()
        && rule.prompt.is_none()
        && rule.task_name.is_none()
        && rule.tag_ids.is_none()
        && rule.run_as_user_id.is_none()
        && rule.agent_type.is_none()
        && rule.inactive_ttl_days.is_none()
    {
        Ok(None)
    } else {
        Ok(Some(rule))
    }
}

#[allow(clippy::too_many_arguments, clippy::option_option)]
fn update_source(
    connection_id: Option<String>,
    source_matcher: Option<String>,
    source_name: Option<String>,
    clear_source_name: bool,
    event_filter: Option<String>,
    adapter_config: Option<Option<serde_json::Value>>,
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
        adapter_config,
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

/// Split "key=value" on the first '='. Errors if absent.
fn parse_kv(raw: &str) -> Result<(String, String), ClientError> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| {
            ClientError::Other(format!("Expected KEY=VALUE, got '{raw}' (missing '=')"))
        })
}

/// JSON-coerce a CLI value: parse as JSON when valid, else a JSON string.
/// e.g. "true"->bool, "5"->number, "^chore\\(deps"->string. To force a string
/// that looks like JSON, quote it: --set k='"5"'.
fn coerce_value(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

/// Apply --unset, then --set, then --append (deterministic order) onto `base`.
/// No-op (returns `base`) when all three are empty. Requires an object base;
/// --append onto a non-array existing key errors.
fn apply_config_ops(
    base: Option<serde_json::Value>,
    sets: Vec<String>,
    appends: Vec<String>,
    unsets: Vec<String>,
) -> Result<Option<serde_json::Value>, ClientError> {
    if sets.is_empty() && appends.is_empty() && unsets.is_empty() {
        return Ok(base);
    }
    let mut obj = base.unwrap_or_else(|| serde_json::json!({}));
    let map = obj.as_object_mut().ok_or_else(|| {
        ClientError::Other("adapter_config must be a JSON object to apply --set/--append/--unset".to_string())
    })?;
    for key in unsets {
        map.remove(&key);
    }
    for raw in sets {
        let (key, value) = parse_kv(&raw)?;
        map.insert(key, coerce_value(&value));
    }
    for raw in appends {
        let (key, value) = parse_kv(&raw)?;
        let entry = map
            .entry(key.clone())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let arr = entry.as_array_mut().ok_or_else(|| {
            ClientError::Other(format!("--append target '{key}' exists but is not an array"))
        })?;
        arr.push(coerce_value(&value));
    }
    Ok(Some(obj))
}

/// Resolve the `adapter_config` field for `triggers update` into the
/// `Option<Option<Value>>` the API expects:
/// - `None` → no change
/// - `Some(None)` → clear the field
/// - `Some(Some(v))` → set/merge to `v`
///
/// `--set/--append/--unset` fetch-merge the trigger's current adapter_config so
/// untouched keys are preserved; `--adapter-config` replaces it wholesale.
#[allow(clippy::option_option)]
async fn resolve_update_adapter_config(
    client: &MoatClient,
    id: &str,
    adapter_config: Option<String>,
    clear: bool,
    sets: Vec<String>,
    appends: Vec<String>,
    unsets: Vec<String>,
) -> Result<Option<Option<serde_json::Value>>, ClientError> {
    let has_ops = !sets.is_empty() || !appends.is_empty() || !unsets.is_empty();
    if clear {
        if adapter_config.is_some() || has_ops {
            return Err(ClientError::Other(
                "--clear-adapter-config cannot be combined with --adapter-config/--set/--append/--unset".to_string(),
            ));
        }
        return Ok(Some(None)); // clear
    }
    let explicit = parse_json_opt(adapter_config.as_deref())?;
    if !has_ops {
        // today's behavior: replace if given, else no change
        return Ok(explicit.map(Some));
    }
    // ops present: base = explicit when given, else the current config (fetch-merge)
    let base = match explicit {
        Some(v) => Some(v),
        None => fetch_current_adapter_config(client, id).await?,
    };
    Ok(Some(apply_config_ops(base, sets, appends, unsets)?))
}

/// Fetch the current `adapter_config` of an adapter-event trigger so ops can
/// merge onto it. Errors if the trigger isn't an adapter-event trigger.
async fn fetch_current_adapter_config(
    client: &MoatClient,
    id: &str,
) -> Result<Option<serde_json::Value>, ClientError> {
    let trigger = client.api().get_trigger(id).await?;
    match trigger.source {
        TriggerSource::AdapterEvent(source) => Ok(source.adapter_config),
        _ => Err(ClientError::Other(format!(
            "Trigger {id} is not an adapter-event trigger; --set/--append/--unset only apply to adapter_config"
        ))),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_kv_splits_on_first_equals() {
        assert_eq!(parse_kv("a=b").unwrap(), ("a".to_string(), "b".to_string()));
        assert_eq!(
            parse_kv("a=b=c").unwrap(),
            ("a".to_string(), "b=c".to_string())
        );
    }

    #[test]
    fn parse_kv_errors_without_equals() {
        assert!(parse_kv("nope").is_err());
    }

    #[test]
    fn coerce_value_parses_json_else_string() {
        assert_eq!(coerce_value("true"), serde_json::json!(true));
        assert_eq!(coerce_value("5"), serde_json::json!(5));
        assert_eq!(
            coerce_value("^chore\\(deps"),
            serde_json::json!("^chore\\(deps")
        );
        assert_eq!(coerce_value(r#"["a","b"]"#), serde_json::json!(["a", "b"]));
        // quoting forces a JSON-looking value to stay a string
        assert_eq!(coerce_value("\"5\""), serde_json::json!("5"));
    }

    #[test]
    fn apply_config_ops_noop_returns_base_unchanged() {
        let base = Some(serde_json::json!({"ignored_senders": ["bot"]}));
        let out = apply_config_ops(base.clone(), vec![], vec![], vec![]).unwrap();
        assert_eq!(out, base);
    }

    #[test]
    fn apply_config_ops_set_creates_and_overwrites() {
        let out = apply_config_ops(None, v(&["a=1"]), vec![], vec![]).unwrap();
        assert_eq!(out, Some(serde_json::json!({"a": 1})));
        let out = apply_config_ops(out, v(&["a=2"]), vec![], vec![]).unwrap();
        assert_eq!(out, Some(serde_json::json!({"a": 2})));
    }

    #[test]
    fn apply_config_ops_append_creates_array_then_pushes() {
        let out = apply_config_ops(
            None,
            vec![],
            v(&["ignored_title_patterns=^chore\\(deps"]),
            vec![],
        )
        .unwrap();
        assert_eq!(
            out,
            Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps"]}))
        );
        let out = apply_config_ops(out, vec![], v(&["ignored_title_patterns=^deps[:(]"]), vec![])
            .unwrap();
        assert_eq!(
            out,
            Some(serde_json::json!({"ignored_title_patterns": ["^chore\\(deps", "^deps[:(]"]}))
        );
    }

    #[test]
    fn apply_config_ops_append_onto_non_array_errors() {
        let base = Some(serde_json::json!({"a": "scalar"}));
        let err = apply_config_ops(base, vec![], v(&["a=x"]), vec![]);
        assert!(err.is_err());
    }

    #[test]
    fn apply_config_ops_unset_removes() {
        let base = Some(serde_json::json!({"a": 1, "b": 2}));
        let out = apply_config_ops(base, vec![], vec![], v(&["a"])).unwrap();
        assert_eq!(out, Some(serde_json::json!({"b": 2})));
    }

    #[test]
    fn apply_config_ops_non_object_base_errors() {
        let base = Some(serde_json::json!(["not", "an", "object"]));
        let err = apply_config_ops(base, v(&["a=1"]), vec![], vec![]);
        assert!(err.is_err());
    }

    #[test]
    fn apply_config_ops_preserves_untouched_keys() {
        let base = Some(serde_json::json!({"ignored_senders": ["bot"]}));
        let out = apply_config_ops(
            base,
            vec![],
            v(&["ignored_title_patterns=^chore\\(deps"]),
            vec![],
        )
        .unwrap();
        assert_eq!(
            out,
            Some(serde_json::json!({
                "ignored_senders": ["bot"],
                "ignored_title_patterns": ["^chore\\(deps"],
            }))
        );
    }

    #[tokio::test]
    async fn resolve_update_clear_with_ops_errors() {
        // clear is checked first, so this short-circuits before any fetch.
        let client = MoatClient::new_unauthenticated().unwrap();
        let err = resolve_update_adapter_config(
            &client,
            "trigger-1",
            None,
            true,
            v(&["a=1"]),
            vec![],
            vec![],
        )
        .await;
        assert!(err.is_err());
    }
}

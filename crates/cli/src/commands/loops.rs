//! Loop commands.
//!
//! A loop is the standing definition of recurring agent work, fired by a
//! subscription (adapter events) or a schedule (cron).

use std::io::IsTerminal;

use clap::Subcommand;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode, Scope};
use moatless_api_schemas::loops::{
    ActivateLoopRequest, CreateLoopConfig, CreateLoopRequest, CreateLoopSource, CreateSchedule,
    CreateSubscription, LoopKind, LoopSource, UpdateLoopConfig, UpdateLoopRequest,
    UpdateLoopSource, UpdateSchedule, UpdateSubscription,
};

use super::adapters::{parse_adapter_kind, parse_json_opt, print_or_json};
use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum LoopsAction {
    /// List loops
    List {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        adapter: Option<String>,
    },
    /// Create a subscription loop (fired by adapter events)
    ///
    /// Adapter-specific knobs live in a free-form `adapter_config` JSON object.
    /// Set them with repeatable ops instead of hand-writing JSON:
    ///   --set KEY=VALUE     set/overwrite a field (VALUE is JSON if valid, else a string)
    ///   --append KEY=VALUE  push onto an array field (creates it if absent)
    ///   --unset KEY         remove a field
    /// or pass the whole object verbatim with --adapter-config '<json>'.
    ///
    /// Known github_pr keys: ignored_senders (string[]), ignored_title_patterns
    /// (case-insensitive title regexes, string[]), ignored_events (string[]),
    /// close_task_on_close, notify_on_success.
    ///
    /// Events are named by --event-filter (an allowlist; empty = all) or
    /// ignored_events (a denylist, which wins). Names: pull_request,
    /// issue_comment, pull_request_review_comment, pull_request_review,
    /// check_run, check_suite, plus the two signals this adapter derives —
    /// merge_conflict and post_merge. Ignoring a derived signal skips its
    /// GitHub call entirely and keeps the events it is derived from.
    /// The backend validates values (e.g. compiles each regex) — this command does not.
    ///
    /// Examples:
    ///   # skip dependency PRs by title
    ///   moat loops create-subscription --adapter github --connection-id <c> \
    ///     --source-matcher owner/repo --repository-id <r> \
    ///     --append ignored_title_patterns='^chore\(deps' \
    ///     --append ignored_title_patterns='^deps[:(]'
    ///
    ///   # force a JSON-looking value to stay a string by quoting it
    ///   --set some_key='"123"'
    #[command(alias = "create-adapter-event")]
    CreateSubscription {
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
        config: CreateConfigArgs,
        /// Human-readable name for the loop
        #[arg(long)]
        name: String,
    },
    /// Create a schedule loop
    CreateSchedule {
        #[arg(long)]
        cron_expression: String,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        message_template: String,
        #[command(flatten)]
        config: CreateConfigArgs,
        /// Human-readable name for the loop
        #[arg(long)]
        name: String,
        /// Create it paused, so it fires only once you activate it
        #[arg(long)]
        active: Option<bool>,
    },
    /// Update loop base, config, or source fields
    ///
    /// Adapter-specific knobs live in a free-form `adapter_config` JSON object.
    /// Set them with repeatable ops instead of hand-writing JSON:
    ///   --set KEY=VALUE     set/overwrite a field (VALUE is JSON if valid, else a string)
    ///   --append KEY=VALUE  push onto an array field (creates it if absent)
    ///   --unset KEY         remove a field
    /// or pass the whole object verbatim with --adapter-config '<json>'.
    ///
    /// --set/--append/--unset fetch-merge the loop's current adapter_config (other
    /// keys are preserved); --adapter-config replaces it wholesale; --clear-adapter-config
    /// removes it.
    ///
    /// Known github_pr keys: ignored_senders (string[]), ignored_title_patterns
    /// (case-insensitive title regexes, string[]), ignored_events (string[]),
    /// close_task_on_close, notify_on_success.
    ///
    /// Events are named by --event-filter (an allowlist; empty = all) or
    /// ignored_events (a denylist, which wins). Names: pull_request,
    /// issue_comment, pull_request_review_comment, pull_request_review,
    /// check_run, check_suite, plus the two signals this adapter derives —
    /// merge_conflict and post_merge. Ignoring a derived signal skips its
    /// GitHub call entirely and keeps the events it is derived from.
    /// The backend validates values (e.g. compiles each regex) — this command does not.
    ///
    /// Examples:
    ///   # append a title pattern, keeping other keys (e.g. ignored_senders)
    ///   moat loops update --id <id> --append ignored_title_patterns='^build\(deps'
    ///
    ///   # force a JSON-looking value to stay a string by quoting it
    ///   --set some_key='"123"'
    Update {
        #[arg(long)]
        id: String,
        /// Rename the loop. A loop always has a name, so it cannot be cleared.
        #[arg(long)]
        name: Option<String>,
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
    },
    /// Activate a loop and atomically select its execution user
    Activate {
        #[arg(long)]
        id: String,
        #[arg(long)]
        run_as_user_id: String,
    },
    /// Pause a loop while preserving its selected execution user
    Pause {
        #[arg(long)]
        id: String,
    },
    /// Resume a paused loop with its existing execution user
    Resume {
        #[arg(long)]
        id: String,
    },
    /// Override Git configuration so repository sync skips this loop
    Override {
        #[arg(long)]
        id: String,
    },
    /// Discard an override or restore a tombstone from Git
    ResetGit {
        #[arg(long)]
        id: String,
    },
    /// Delete a loop
    Delete {
        #[arg(long)]
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args)]
pub struct CreateConfigArgs {
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
    /// Per-loop inactivity TTL in days. Tasks spawned by this loop
    /// auto-close after this many days of inactivity (0 = never close).
    /// Omit to inherit the platform-wide default (TASK_INACTIVE_TIMEOUT_DAYS).
    #[arg(long)]
    inactive_ttl_days: Option<i32>,
}

pub async fn run_loops(
    client: &MoatClient,
    action: LoopsAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        LoopsAction::List { kind, adapter } => {
            let kind = kind.as_deref().map(parse_loop_kind).transpose()?;
            let adapter = adapter.as_deref().map(parse_adapter_kind).transpose()?;
            let items = client
                .api()
                .list_loops(
                    kind.as_ref().map(LoopKind::as_str),
                    adapter.as_ref().map(AdapterKind::as_str),
                )
                .await?;
            print_or_json(&items, json);
        }
        LoopsAction::CreateSubscription {
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
            config,
            name,
        } => {
            let base = parse_json_opt(adapter_config.as_deref())?;
            let adapter_config = apply_config_ops(base, config_set, config_append, config_unset)?;
            let req = CreateLoopRequest {
                name,
                kind: LoopKind::AdapterEvent,
                active: None,
                scope: None,
                config: create_config(config)?,
                source: CreateLoopSource::AdapterEvent(CreateSubscription {
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
            let created = client.api().create_loop(&req).await?;
            print_or_json(&created, json);
        }
        LoopsAction::CreateSchedule {
            cron_expression,
            timezone,
            message_template,
            config,
            name,
            active,
        } => {
            let req = CreateLoopRequest {
                name,
                kind: LoopKind::Schedule,
                active,
                scope: None,
                config: create_config(config)?,
                source: CreateLoopSource::Schedule(CreateSchedule {
                    cron_expression,
                    timezone,
                    message_template,
                }),
            };
            let created = client.api().create_loop(&req).await?;
            print_or_json(&created, json);
        }
        LoopsAction::Update {
            id,
            name,
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
            let config = update_config(
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
            let req = UpdateLoopRequest {
                name,
                scope,
                config,
                source,
            };
            let updated = client.api().update_loop(&id, &req).await?;
            print_or_json(&updated, json);
        }
        LoopsAction::Activate { id, run_as_user_id } => {
            let updated = client
                .api()
                .activate_loop(&id, &ActivateLoopRequest { run_as_user_id })
                .await?;
            print_or_json(&updated, json);
        }
        LoopsAction::Pause { id } => {
            let updated = client.api().pause_loop(&id).await?;
            print_or_json(&updated, json);
        }
        LoopsAction::Resume { id } => {
            let updated = client.api().resume_loop(&id).await?;
            print_or_json(&updated, json);
        }
        LoopsAction::Override { id } => {
            let updated = client.api().override_loop(&id).await?;
            print_or_json(&updated, json);
        }
        LoopsAction::ResetGit { id } => {
            let result = client.api().reset_git_loop(&id).await?;
            print_or_json(&result, json);
        }
        LoopsAction::Delete { id, yes } => {
            let loop_ = client.api().get_loop(&id).await?;
            let restorable = loop_.source_repository_id.is_some();
            if !yes {
                confirm_delete(&id, restorable)?;
            }
            client.api().delete_loop(&id).await?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else if restorable {
                println!(
                    "Deleted loop {id}; its Git tombstone can be restored with `moat loops reset-git --id {id}`."
                );
            } else {
                println!("Deleted loop {id} permanently.");
            }
        }
    }
    Ok(())
}

fn create_config(args: CreateConfigArgs) -> Result<CreateLoopConfig, ClientError> {
    Ok(CreateLoopConfig {
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

// reason: the loop-config update API exposes a clear/value pair per
// nullable field, so the bool count is intrinsic to the surface.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn update_config(
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
    agent_type: Option<String>,
    inactive_ttl_days: Option<i32>,
    clear_inactive_ttl_days: bool,
) -> Result<Option<UpdateLoopConfig>, ClientError> {
    let tag_ids = fold_tag_ids(tag_ids, clear_tags)?;
    let config = UpdateLoopConfig {
        repository_id,
        routing_mode,
        ongoing_task_id: fold_clearable(ongoing_task_id, clear_ongoing_task_id, "ongoing-task-id")?,
        prompt: fold_clearable(prompt, clear_prompt, "prompt")?,
        task_name: fold_clearable(task_name, clear_task_name, "task-name")?,
        tag_ids,
        agent_type,
        inactive_ttl_days: fold_clearable(
            inactive_ttl_days,
            clear_inactive_ttl_days,
            "inactive-ttl-days",
        )?,
    };
    if config.repository_id.is_none()
        && config.routing_mode.is_none()
        && config.ongoing_task_id.is_none()
        && config.prompt.is_none()
        && config.task_name.is_none()
        && config.tag_ids.is_none()
        && config.agent_type.is_none()
        && config.inactive_ttl_days.is_none()
    {
        Ok(None)
    } else {
        Ok(Some(config))
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
) -> Result<Option<UpdateLoopSource>, ClientError> {
    let adapter_source = UpdateSubscription {
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

    let schedule_source = UpdateSchedule {
        cron_expression,
        timezone,
        message_template,
    };
    let has_schedule_source = schedule_source.cron_expression.is_some()
        || schedule_source.timezone.is_some()
        || schedule_source.message_template.is_some();

    match (has_adapter_source, has_schedule_source) {
        (false, false) => Ok(None),
        (true, false) => Ok(Some(UpdateLoopSource::AdapterEvent(adapter_source))),
        (false, true) => Ok(Some(UpdateLoopSource::Schedule(schedule_source))),
        (true, true) => Err(ClientError::Other(
            "Subscription and schedule source fields cannot be updated in the same command"
                .to_string(),
        )),
    }
}

fn confirm_delete(id: &str, restorable: bool) -> Result<(), ClientError> {
    use std::io::Write;

    if !std::io::stdin().is_terminal() {
        return Err(ClientError::Other(
            "Refusing to delete without --yes in non-interactive mode".to_string(),
        ));
    }
    let consequence = if restorable {
        "Its Git tombstone remains restorable."
    } else {
        "This cannot be undone."
    };
    print!("Delete loop {id}? {consequence} [y/N] ");
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

fn parse_loop_kind(value: &str) -> Result<LoopKind, ClientError> {
    LoopKind::try_from(value).map_err(ClientError::Other)
}

fn parse_routing_mode(value: &str) -> Result<RoutingMode, ClientError> {
    RoutingMode::try_from(value).map_err(ClientError::Other)
}

/// Split "key=value" on the first '='. Errors if absent.
fn parse_kv(raw: &str) -> Result<(String, String), ClientError> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| ClientError::Other(format!("Expected KEY=VALUE, got '{raw}' (missing '=')")))
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
        ClientError::Other(
            "adapter_config must be a JSON object to apply --set/--append/--unset".to_string(),
        )
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
            ClientError::Other(format!(
                "--append target '{key}' exists but is not an array"
            ))
        })?;
        arr.push(coerce_value(&value));
    }
    Ok(Some(obj))
}

/// Resolve the `adapter_config` field for `loops update` into the
/// `Option<Option<Value>>` the API expects:
/// - `None` → no change
/// - `Some(None)` → clear the field
/// - `Some(Some(v))` → set/merge to `v`
///
/// `--set/--append/--unset` fetch-merge the loop's current adapter_config so
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

/// Fetch the current `adapter_config` of a subscription loop so ops can
/// merge onto it. Errors if the loop isn't a subscription loop.
async fn fetch_current_adapter_config(
    client: &MoatClient,
    id: &str,
) -> Result<Option<serde_json::Value>, ClientError> {
    let loop_def = client.api().get_loop(id).await?;
    match loop_def.source {
        LoopSource::AdapterEvent(source) => Ok(source.adapter_config),
        _ => Err(ClientError::Other(format!(
            "Loop {id} is not a subscription loop; --set/--append/--unset only apply to adapter_config"
        ))),
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
        "user" => Ok(Scope::User),
        other => Err(ClientError::Other(format!(
            "Invalid scope '{other}'. Expected global or user"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(ToString::to_string).collect()
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
        let out = apply_config_ops(
            out,
            vec![],
            v(&["ignored_title_patterns=^deps[:(]"]),
            vec![],
        )
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
            "loop-1",
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

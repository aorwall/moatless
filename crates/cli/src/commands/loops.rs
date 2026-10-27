//! Loop commands.
//!
//! A loop is the standing definition of recurring agent work, fired by a
//! subscription (adapter events) or a schedule (cron).

use std::io::IsTerminal;

use clap::Subcommand;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode, Scope};
use moatless_api_schemas::loops::{
    ActivateLoopRequest, CreateLoopConfig, CreateLoopRequest, CreateLoopSource, CreateSchedule,
    CreateSubscription, Loop, LoopConfigSource, LoopExecutionState, LoopKind, LoopRecentTaskItem,
    LoopSource, UpdateLoopConfig, UpdateLoopRequest, UpdateLoopSource, UpdateSchedule,
    UpdateSubscription,
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
    /// List the tasks a loop most recently spawned, newest first
    Tasks {
        #[arg(long)]
        id: String,
        /// Max rows, default 10, capped at 50 by the server
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Create a subscription loop (fired by adapter events).
    ///
    /// `adapter_config` is free-form JSON (--set/--append/--unset or
    /// --adapter-config); --event-filter allowlists, `ignored_events`
    /// denylists and wins; ignoring a derived signal
    /// (`merge_conflict`/`post_merge`/`pr_merged`) skips only that signal.
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
        /// Set an `adapter_config` field: KEY=VALUE. VALUE is parsed as JSON
        /// when valid (`true`, `5`, `["a"]`, `{"k":1}`), otherwise treated as a
        /// string. Repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        config_set: Vec<String>,
        /// Append a value to an array field in `adapter_config`: KEY=VALUE.
        /// Creates the array if absent. Repeatable. e.g. --append
        /// `ignored_title_patterns`='^chore\(deps'
        #[arg(long = "append", value_name = "KEY=VALUE")]
        config_append: Vec<String>,
        /// Remove a field from `adapter_config`. Repeatable. e.g. --unset
        /// `ignored_senders`
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
    /// Update loop base, config, or source fields.
    ///
    /// --set/--append/--unset fetch-merge `adapter_config` (other keys
    /// preserved); --adapter-config replaces it wholesale,
    /// --clear-adapter-config removes it. Same event-filter/`ignored_events`
    /// precedence and derived-signal exception as `create-subscription`.
    Update {
        #[arg(long)]
        id: String,
        /// Rename the loop. A loop always has a name, so it cannot be cleared.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        workspace_id: Option<String>,
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
        #[arg(long = "skill")]
        skills: Vec<String>,
        #[arg(long)]
        clear_skills: bool,
        /// Memory key, or key prefix ending in `*`, read from the loop's scope
        /// into each task's prompt. Repeat for several; replaces the list.
        #[arg(long = "memory-preload", value_name = "PATTERN")]
        memory_preload: Vec<String>,
        #[arg(long)]
        clear_memory_preload: bool,
        #[arg(long)]
        agent_profile: Option<String>,
        /// Model the loop's tasks run on, from those the agent profile offers
        /// (e.g. `opus`, `fable` for `claude-code`). The backend rejects a
        /// model the agent profile does not offer.
        #[arg(long)]
        model: Option<String>,
        /// Drop the model override, so tasks run on the platform default for
        /// the agent profile.
        #[arg(long)]
        clear_model: bool,
        #[arg(long, value_name = "DAYS", alias = "inactive-ttl-days")]
        close_after_settled_days: Option<i32>,
        #[arg(long, alias = "clear-inactive-ttl-days")]
        clear_close_after_settled_days: bool,
        #[arg(long, value_name = "HOURS")]
        settle_after_hours: Option<i32>,
        #[arg(long)]
        clear_settle_after_hours: bool,
        /// Extra node labels for this loop's sandbox pods, as a JSON object,
        /// e.g. `{"workload":"moatless-od"}`.
        #[arg(long, value_name = "JSON")]
        node_selector: Option<String>,
        /// Drop the node-selector override, so sandboxes fall back to the
        /// workspace's own placement.
        #[arg(long)]
        clear_node_selector: bool,
        /// Stop this loop's sandbox as soon as the agent run finishes, instead
        /// of holding the pod for the idle TTL.
        #[arg(long)]
        stop_when_done: Option<bool>,
        /// Re-run a task's turn once on the other agent (codex or claude-code)
        /// when its provider will not serve it (usage limit, rate limit, outage).
        #[arg(long)]
        fallback_to_other_agent: Option<bool>,
        /// Most seconds to hold inbound adapter events for one binding so those
        /// arriving together reach the task as one message; a lone event leaves
        /// after a short quiet period. 0 turns buffering off.
        #[arg(long)]
        message_buffer_seconds: Option<i32>,
        /// Drop the buffering override, so the adapter kind's deployment
        /// default applies again.
        #[arg(long)]
        clear_message_buffer_seconds: bool,
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
        /// Set an `adapter_config` field: KEY=VALUE. VALUE is parsed as JSON
        /// when valid (`true`, `5`, `["a"]`, `{"k":1}`), otherwise treated as a
        /// string. Repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        config_set: Vec<String>,
        /// Append a value to an array field in `adapter_config`: KEY=VALUE.
        /// Creates the array if absent. Repeatable. e.g. --append
        /// `ignored_title_patterns`='^chore\(deps'
        #[arg(long = "append", value_name = "KEY=VALUE")]
        config_append: Vec<String>,
        /// Remove a field from `adapter_config`. Repeatable. e.g. --unset
        /// `ignored_senders`
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
    /// The Workspace every task this loop spawns runs in.
    #[arg(long)]
    workspace_id: String,
    #[arg(long, default_value = "by_subject")]
    routing_mode: String,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    task_name: Option<String>,
    #[arg(long)]
    run_as_user_id: Option<String>,
    #[arg(long, default_value = "claude-code")]
    agent_profile: String,
    /// Model the loop's tasks run on, from those the agent profile offers (e.g.
    /// `opus`, `fable` for `claude-code`). Omit to inherit the platform default
    /// for the agent profile.
    #[arg(long)]
    model: Option<String>,
    #[arg(long = "tag")]
    tag_ids: Vec<String>,
    /// Skills every task this loop spawns activates on each fresh agent
    /// session. Repeat for several.
    #[arg(long = "skill")]
    skills: Vec<String>,
    /// Memory key, or key prefix ending in `*`, read from the loop's scope into
    /// each task's prompt, like `memory.preload` in a LOOP.md. Repeat for
    /// several.
    #[arg(long = "memory-preload", value_name = "PATTERN")]
    memory_preload: Vec<String>,
    /// Tasks spawned by this loop auto-close this many days after they are
    /// settled (0 = never close). Omit to inherit the platform-wide default
    /// (`TASK_CLOSE_AFTER_SETTLED_DAYS`).
    #[arg(long, alias = "inactive-ttl-days")]
    close_after_settled_days: Option<i32>,
    /// Tasks spawned by this loop are settled after this many idle hours
    /// (0 = never settle). Omit for the loop default of 1.
    #[arg(long)]
    settle_after_hours: Option<i32>,
    /// Extra node labels for this loop's sandbox pods, as a JSON object, e.g.
    /// `{"workload":"moatless-od"}`. Outranks the workspace's own value.
    #[arg(long, value_name = "JSON")]
    node_selector: Option<String>,
    /// Stop this loop's sandbox as soon as the agent run finishes, instead of
    /// holding the pod for the idle TTL. Pairs with a dedicated node pool: the
    /// node goes idle in seconds and can scale down.
    #[arg(long)]
    stop_when_done: Option<bool>,
    /// Re-run a task's turn once on the other agent (codex or claude-code) when
    /// its provider will not serve it (usage limit, rate limit, outage).
    /// Omitted is on.
    #[arg(long)]
    fallback_to_other_agent: Option<bool>,
    /// Most seconds to hold inbound adapter events for one binding so those
    /// arriving together reach the task as one message; a lone event leaves
    /// after a short quiet period. 0 turns buffering off; omitted inherits the
    /// adapter kind's deployment default.
    #[arg(long)]
    message_buffer_seconds: Option<i32>,
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
                .list_loops(kind.map(LoopKind::as_str), adapter.map(AdapterKind::as_str))
                .await?;
            output::emit(json, &items, || format_loop_list(&items));
        }
        LoopsAction::Tasks { id, limit } => {
            let items = client.api().list_loop_recent_tasks(&id, limit).await?;
            output::emit(json, &items, || format_loop_task_list(&items));
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
            workspace_id,
            routing_mode,
            ongoing_task_id,
            clear_ongoing_task_id,
            prompt,
            clear_prompt,
            task_name,
            clear_task_name,
            tag_ids,
            clear_tags,
            skills,
            clear_skills,
            memory_preload,
            clear_memory_preload,
            agent_profile,
            model,
            clear_model,
            close_after_settled_days,
            clear_close_after_settled_days,
            settle_after_hours,
            clear_settle_after_hours,
            node_selector,
            clear_node_selector,
            stop_when_done,
            fallback_to_other_agent,
            message_buffer_seconds,
            clear_message_buffer_seconds,
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
                workspace_id,
                routing_mode,
                ongoing_task_id,
                clear_ongoing_task_id,
                prompt,
                clear_prompt,
                task_name,
                clear_task_name,
                tag_ids,
                clear_tags,
                skills,
                clear_skills,
                memory_preload,
                clear_memory_preload,
                agent_profile,
                model,
                clear_model,
                close_after_settled_days,
                clear_close_after_settled_days,
                settle_after_hours,
                clear_settle_after_hours,
                node_selector,
                clear_node_selector,
                stop_when_done,
                fallback_to_other_agent,
                message_buffer_seconds,
                clear_message_buffer_seconds,
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

fn parse_node_selector(
    raw: Option<&str>,
) -> Result<Option<std::collections::BTreeMap<String, String>>, ClientError> {
    raw.map(|s| {
        serde_json::from_str(s)
            .map_err(|e| ClientError::Other(format!("Invalid --node-selector: {e}")))
    })
    .transpose()
}

fn create_config(args: CreateConfigArgs) -> Result<CreateLoopConfig, ClientError> {
    Ok(CreateLoopConfig {
        workspace_id: args.workspace_id,
        routing_mode: parse_routing_mode(&args.routing_mode)?,
        ongoing_task_id: None,
        task_name: args.task_name,
        prompt: args.prompt,
        tag_ids: Some(args.tag_ids),
        memory_preload: Some(args.memory_preload),
        skills: Some(args.skills),
        run_as_user_id: args.run_as_user_id,
        agent_profile: Some(args.agent_profile),
        model: args.model,
        close_after_settled_days: args.close_after_settled_days,
        settle_after_hours: args.settle_after_hours,
        node_selector: parse_node_selector(args.node_selector.as_deref())?,
        stop_when_done: args.stop_when_done,
        message_buffer_seconds: args.message_buffer_seconds,
        fallback_to_other_agent: args.fallback_to_other_agent,
    })
}

// reason: the loop-config update API exposes a clear/value pair per
// nullable field, so the bool count is intrinsic to the surface.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn update_config(
    workspace_id: Option<String>,
    routing_mode: Option<RoutingMode>,
    ongoing_task_id: Option<String>,
    clear_ongoing_task_id: bool,
    prompt: Option<String>,
    clear_prompt: bool,
    task_name: Option<String>,
    clear_task_name: bool,
    tag_ids: Vec<String>,
    clear_tags: bool,
    skills: Vec<String>,
    clear_skills: bool,
    memory_preload: Vec<String>,
    clear_memory_preload: bool,
    agent_profile: Option<String>,
    model: Option<String>,
    clear_model: bool,
    close_after_settled_days: Option<i32>,
    clear_close_after_settled_days: bool,
    settle_after_hours: Option<i32>,
    clear_settle_after_hours: bool,
    node_selector: Option<String>,
    clear_node_selector: bool,
    stop_when_done: Option<bool>,
    fallback_to_other_agent: Option<bool>,
    message_buffer_seconds: Option<i32>,
    clear_message_buffer_seconds: bool,
) -> Result<Option<UpdateLoopConfig>, ClientError> {
    let tag_ids = fold_list(tag_ids, clear_tags, "tag", "clear-tags")?;
    let skills = fold_list(skills, clear_skills, "skill", "clear-skills")?;
    let memory_preload = fold_list(
        memory_preload,
        clear_memory_preload,
        "memory-preload",
        "clear-memory-preload",
    )?;
    let config = UpdateLoopConfig {
        workspace_id,
        routing_mode,
        ongoing_task_id: fold_clearable(ongoing_task_id, clear_ongoing_task_id, "ongoing-task-id")?,
        prompt: fold_clearable(prompt, clear_prompt, "prompt")?,
        task_name: fold_clearable(task_name, clear_task_name, "task-name")?,
        tag_ids,
        memory_preload,
        skills,
        agent_profile,
        model: fold_clearable(model, clear_model, "model")?,
        close_after_settled_days: fold_clearable(
            close_after_settled_days,
            clear_close_after_settled_days,
            "close-after-settled-days",
        )?,
        settle_after_hours: fold_clearable(
            settle_after_hours,
            clear_settle_after_hours,
            "settle-after-hours",
        )?,
        node_selector: fold_clearable(
            parse_node_selector(node_selector.as_deref())?,
            clear_node_selector,
            "node-selector",
        )?,
        stop_when_done,
        message_buffer_seconds: fold_clearable(
            message_buffer_seconds,
            clear_message_buffer_seconds,
            "message-buffer-seconds",
        )?,
        fallback_to_other_agent,
    };
    if config.workspace_id.is_none()
        && config.routing_mode.is_none()
        && config.ongoing_task_id.is_none()
        && config.prompt.is_none()
        && config.task_name.is_none()
        && config.tag_ids.is_none()
        && config.skills.is_none()
        && config.memory_preload.is_none()
        && config.agent_profile.is_none()
        && config.model.is_none()
        && config.close_after_settled_days.is_none()
        && config.settle_after_hours.is_none()
        && config.node_selector.is_none()
        && config.stop_when_done.is_none()
        && config.message_buffer_seconds.is_none()
        && config.fallback_to_other_agent.is_none()
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

/// JSON-coerce a CLI value: parse as JSON when valid, else a JSON string. e.g.
/// "true"->bool, "5"->number, "^chore\\(deps"->string. To force a string that
/// looks like JSON, quote it: --set k='"5"'.
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

/// Resolves `adapter_config` for `loops update` to the `Option<Option<Value>>`
/// the API expects: `None` = no change, `Some(None)` = clear, `Some(Some(v))` =
/// set/merge to `v`.
///
/// `--set/--append/--unset` fetch-merge the current config (other keys kept);
/// `--adapter-config` replaces it wholesale.
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

/// Fetch the current `adapter_config` of a subscription loop so ops can merge
/// onto it. Errors if the loop isn't a subscription loop.
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

/// Fold a repeatable list flag and its `--clear-*` companion into the patch
/// value: `None` leaves the field alone, `Some(vec![])` empties it.
fn fold_list(
    values: Vec<String>,
    clear: bool,
    value_flag: &str,
    clear_flag: &str,
) -> Result<Option<Vec<String>>, ClientError> {
    match (values.is_empty(), clear) {
        (false, true) => Err(ClientError::Other(format!(
            "--{value_flag} and --{clear_flag} are mutually exclusive"
        ))),
        (false, false) => Ok(Some(values)),
        (true, true) => Ok(Some(vec![])),
        (true, false) => Ok(None),
    }
}

/// Whether a row keeps a `sourceConfigPath` it no longer reads.
///
/// An overridden Loop keeps the path and stops reading it, so editing that
/// `LOOP.md` and merging the pull request changes nothing that runs. The only
/// exit is `moat loops reset-git`.
fn is_overridden(l: &Loop) -> bool {
    l.config_source == LoopConfigSource::Manual && l.source_config_path.is_some()
}

/// Why a row is or is not running.
///
/// `active` alone collapses two states with different fixes: an administrator
/// paused this, or nobody has approved it yet.
fn state_cell(l: &Loop) -> &'static str {
    match l.execution_state {
        LoopExecutionState::Active => "active",
        LoopExecutionState::Paused => "paused",
        LoopExecutionState::AwaitingApproval => "awaiting-approval",
    }
}

/// How a row's stored configuration relates to its Git declaration.
fn config_source_cell(l: &Loop) -> &'static str {
    match l.config_source {
        LoopConfigSource::Git => "git",
        _ if is_overridden(l) => "manual (overridden)",
        LoopConfigSource::Manual => "manual",
    }
}

fn format_loop_list(loops: &[Loop]) {
    let rows: Vec<Vec<String>> = loops
        .iter()
        .map(|l| {
            vec![
                l.id.clone(),
                l.name.clone(),
                l.kind.as_str().to_string(),
                state_cell(l).to_string(),
                config_source_cell(l).to_string(),
            ]
        })
        .collect();
    output::print_table(&["ID", "NAME", "KIND", "STATE", "CONFIG"], &rows);

    let overridden: Vec<&Loop> = loops.iter().filter(|l| is_overridden(l)).collect();
    if !overridden.is_empty() {
        println!();
        println!(
            "{} loop(s) ignore their Git declaration until `moat loops reset-git --id <id>`:",
            overridden.len()
        );
        for l in overridden {
            let path = l.source_config_path.as_deref().unwrap_or("?");
            println!("  {}  {}  (declared in {path})", l.id, l.name);
        }
    }
}

/// The return channel a spawned task answers on, or `-` when it has none —
/// see [`LoopRecentTaskItem::binding`]. `-` is a task nothing routes back, not
/// a task with an unnamed channel.
fn binding_cell(item: &LoopRecentTaskItem) -> String {
    item.binding.as_ref().map_or_else(
        || "-".to_string(),
        |b| format!("{}:{}", b.adapter_kind.as_str(), b.subject_key),
    )
}

fn format_loop_task_list(items: &[LoopRecentTaskItem]) {
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            vec![
                item.task.task_id.clone(),
                item.task.status.as_str().to_string(),
                item.task.created_at.clone(),
                binding_cell(item),
                item.task.title.clone(),
            ]
        })
        .collect();
    output::print_table(&["TASK ID", "STATUS", "CREATED", "BINDING", "TITLE"], &rows);
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

    fn model_update(
        model: Option<&str>,
        clear: bool,
    ) -> Result<Option<UpdateLoopConfig>, ClientError> {
        update_config(
            None,
            None,
            None,
            false,
            None,
            false,
            None,
            false,
            vec![],
            false,
            vec![],
            false,
            vec![],
            false,
            None,
            model.map(ToString::to_string),
            clear,
            None,
            false,
            None,
            false,
            None,
            false,
            None,
            None,
            None,
            false,
        )
    }

    #[test]
    fn model_sets_clears_and_refuses_both() {
        let set = model_update(Some("fable"), false)
            .unwrap()
            .expect("--model is an update on its own");
        assert_eq!(set.model, Some(Some("fable".to_string())));

        let cleared = model_update(None, true)
            .unwrap()
            .expect("--clear-model is an update on its own");
        assert_eq!(cleared.model, Some(None));

        assert!(model_update(Some("fable"), true).is_err());
        assert!(model_update(None, false).unwrap().is_none());
    }

    /// An absent binding renders `-`, and must not fail to parse: the field is
    /// `null` on the wire, not omitted.
    #[test]
    fn a_task_with_no_binding_names_no_channel() {
        let items: Vec<LoopRecentTaskItem> = serde_json::from_value(serde_json::json!([
            {
                "task": {
                    "taskId": "t-1", "shortId": "t1", "title": "Review PR",
                    "status": "open", "createdAt": "2026-09-18T06:05:07Z"
                },
                "binding": {
                    "id": "b-1", "taskId": "t-1", "adapterKind": "github_pr",
                    "connectionId": "c-1", "loopId": "l-1", "subjectKind": "pull_request",
                    "subjectKey": "soaplabs/moatless#959", "destinationConfig": {},
                    "metadata": {}, "lastExternalEventId": null, "origin": "adapter_event",
                    "createdAt": "2026-09-18T06:00:00Z", "lastEventAt": "2026-09-18T06:05:00Z"
                }
            },
            {
                "task": {
                    "taskId": "t-2", "shortId": null, "title": "Nightly sweep",
                    "status": "closed", "createdAt": "2026-09-17T02:00:00Z"
                },
                "binding": null
            }
        ]))
        .expect("the wire shape the CLI reads");
        assert_eq!(binding_cell(&items[0]), "github_pr:soaplabs/moatless#959");
        assert_eq!(binding_cell(&items[1]), "-");
    }

    #[tokio::test]
    async fn resolve_update_clear_with_ops_errors() {
        // clear is checked first, so this short-circuits before any fetch.
        let client = MoatClient::new_unauthenticated(
            "http://localhost:8080",
            crate::config::HttpConfig::default(),
        )
        .unwrap();
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

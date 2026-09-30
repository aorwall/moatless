//! The `moat tasks` subcommand surface, and the dispatch into the modules
//! below.

use super::create::{
    require_a_workspace_source, upload_context_files, validate_context_files_have_message,
};
use super::files::{FileAction, run_file_action};
use super::format::{format_task_detail, format_task_list, parse_task_status};
use super::logs::stream_logs;
use super::resolve::{
    resolve_parent_task_id, resolve_repo_id, resolve_same_sandbox, resolve_task_id,
    resolve_workspace_id,
};
use super::sandbox::{
    SandboxOperation, cancel_wake_note, format_sandbox_status, record_wake_note, run_diagnose,
    self_restart_note,
};
use super::scripts::{TaskScriptAction, run_task_script_action};
use crate::client::{ClientError, MoatClient};
use crate::config::Config;
use crate::output;
use clap::Subcommand;
use moatless_api_schemas::enums::CheckoutMode;
use moatless_api_schemas::task::{
    CloseTaskRequest, CreateTaskRequest, ForkTaskRequest, ListTasksQuery,
    UpdateTaskMetadataRequest, UpdateTaskTtlRequest,
};
use moatless_types::agent::{AgentMode, AgentProfile};

#[derive(Subcommand)]
pub enum TaskAction {
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "50")]
        limit: i64,
        /// Skip this many tasks, for walking past the first page
        #[arg(long)]
        offset: Option<i64>,
    },
    /// Get task details
    Get { task_id: String },
    /// Fork a task: branch a new task off it, carrying its conversation and the
    /// agent's own session up to a chosen turn
    Fork {
        /// Task id or short id to branch off
        task_id: String,
        /// The source task's turn the copy stops after. Omit to cut after its
        /// newest completed turn
        #[arg(long)]
        at_turn: Option<i32>,
        /// The fork's first message — what to try instead. Omit to create the
        /// fork idle and send to it later
        #[arg(long)]
        message: Option<String>,
        /// Join the source task's sandbox on its shared checkout instead of
        /// provisioning one from a copy of its workspace
        #[arg(long)]
        same_sandbox: bool,
        /// Agent profile the fork runs, as `moat agents` lists it. Omit to run
        /// the source task's
        #[arg(long)]
        agent_profile: Option<String>,
    },
    /// Create a new task (starts the sandbox and, if --message is provided, the
    /// agent)
    Create {
        /// Repository ID or name. Ignored: the workspace's primary repository
        /// is what the task runs against
        #[arg(long)]
        repo: Option<String>,
        /// Workspace ID (ws_...) or name to run the task in. Required unless a
        /// parent task or a joined sandbox already settles one
        #[arg(long)]
        workspace: Option<String>,
        /// Runs in the same Sandbox as another task (by task id, not sandbox
        /// id), reusing its checkout. Bare `--same-sandbox` means this CLI's
        /// own task.
        ///
        /// A Sandbox materializes exactly one Workspace, so
        /// `--repo`/`--workspace` conflict with it.
        #[arg(
            long,
            value_name = "TASK_ID",
            num_args = 0..=1,
            conflicts_with_all = ["repo", "workspace"]
        )]
        same_sandbox: Option<Option<String>>,
        /// Whether this task shares the joined Sandbox's checkout (`shared`,
        /// the default) or gets its own (`isolated`).
        ///
        // `requires` alone is not enough: clap skips it when the required arg
        // conflicts with one that *is* present, so `--repo x --checkout-mode
        // isolated` would parse and then be silently dropped. Conflicting with
        // the same two args as --same-sandbox makes that combination an error.
        #[arg(
            long,
            value_parser = ["shared", "isolated"],
            requires = "same_sandbox",
            conflicts_with_all = ["repo", "workspace"]
        )]
        checkout_mode: Option<String>,
        #[arg(long)]
        name: Option<String>,
        /// Parent task id. Inside a sandbox this CLI's own task is always the
        /// parent. The backend tells the parent each time the child's turn
        /// completes or errors; the child still sends its result itself
        #[arg(long)]
        parent: Option<String>,
        /// Identity the task's sandbox acts as on a git host — a moatless login
        /// or user id. Admin only, and only a bot user; omit it to run as
        /// yourself.
        #[arg(long, value_name = "USER")]
        run_as: Option<String>,
        #[arg(long)]
        message: Option<String>,
        /// File to upload and attach to `--message`. Repeatable. The new task's
        /// sandbox writes it in before its agent runs.
        #[arg(long = "context-file", alias = "relevant-file", value_name = "PATH")]
        context_files: Vec<String>,
        #[arg(long)]
        branch: Option<String>,
        /// Harness to run the task with. Run `moat agents models` to list
        /// available harnesses and model IDs.
        #[arg(
            long,
            required = true,
            value_parser = clap::builder::PossibleValuesParser::new(AgentProfile::SUPPORTED)
        )]
        agent_profile: String,
        /// Model for the chosen harness, e.g. `haiku` or `gpt-5.5`
        #[arg(long)]
        model: Option<String>,
        /// Runs headless: no tools that wait on a person (`AskUserQuestion`,
        /// Enter/ExitPlanMode).
        ///
        /// Defaults to true for CLI-created tasks, since there's no browser
        /// here to view them or answer a prompt.
        // `num_args`/`default_missing_value` are what make the value optional:
        // a plain `bool` field is a set-true flag in clap, which would leave no
        // way to turn an on-by-default option off.
        #[arg(
            long,
            value_name = "BOOL",
            num_args = 0..=1,
            default_value_t = true,
            default_missing_value = "true"
        )]
        headless: bool,
    },
    /// List child tasks
    Children {
        /// Parent task ID (defaults to current `TASK_ID`)
        task_id: Option<String>,
    },
    /// Start a task
    Start { task_id: String },
    /// Stop a task
    Stop { task_id: String },
    /// Close a task
    Close {
        task_id: String,
        /// Also close all descendant tasks. Required as true or false when the
        /// task has children.
        #[arg(long, value_name = "BOOL", num_args = 1)]
        close_children: Option<bool>,
    },
    /// Reopen a closed task
    Reopen { task_id: String },
    /// Update a task's name, description, model and/or Agent profile
    Update {
        task_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Model for the task's later turns, as `moat agents models` lists it.
        /// Takes effect from the next turn; the running one keeps its model.
        #[arg(long)]
        model: Option<String>,
        /// Agent profile for the task's later turns, as `moat agents` lists
        /// it. Without `--model` the profile's default model runs.
        #[arg(long)]
        agent_profile: Option<String>,
    },
    /// Update a task's TTLs (idle sandbox stop and/or inactivity auto-close)
    Ttl {
        task_id: String,
        /// Idle TTL in minutes (0 = never stop the sandbox)
        #[arg(long)]
        ttl_minutes: Option<i32>,
        /// Inactivity TTL in days (0 = never auto-close the task)
        #[arg(long)]
        inactive_ttl_days: Option<i32>,
    },
    /// Show full task status (environment, agent)
    Status { task_id: String },
    /// Restart sandbox for a task
    Restart {
        task_id: String,
        /// Restart this command's own sandbox. Required there, and ignored for
        /// any other task.
        #[arg(long)]
        yes: bool,
    },
    /// Redeploy sandbox (recreate with fresh repo config, preserves workspace
    /// data)
    Redeploy {
        task_id: String,
        /// Redeploy this command's own sandbox. Required there, and ignored for
        /// any other task.
        #[arg(long)]
        yes: bool,
    },
    /// Stream the sandbox container's logs
    Logs {
        task_id: String,
        /// Fetch logs from previous (crashed/restarted) container instance
        #[arg(long)]
        previous: bool,
        /// Print what is buffered and exit instead of following the stream
        #[arg(long)]
        no_follow: bool,
        /// Number of trailing lines to print
        #[arg(long, default_value = "500")]
        tail: i64,
    },
    /// Diagnose sandbox issues (status + container diagnostics + recent errors)
    Diagnose {
        task_id: String,
        /// Number of error events to show
        #[arg(long, default_value = "10")]
        limit: i64,
    },
    /// Print what the `restic` CLI needs to open a task's workspace repository.
    ///
    /// Platform admins only — the output carries live S3 credentials and the
    /// repository password.
    WorkspaceRepo { task_id: String },
    /// File operations in a task's workspace
    Files {
        task_id: String,
        #[command(subcommand)]
        action: FileAction,
    },
    /// Run a workspace's declared script against this task's sandbox, reporting
    /// a preview URL and opening a browser tab in the T3 UI if it declares a
    /// port.
    ///
    /// Unlike clicking Run in the chat header, this does not start a stopped
    /// sandbox — run `moat tasks start` first.
    Scripts {
        task_id: String,
        #[command(subcommand)]
        action: TaskScriptAction,
    },
}

pub async fn run(
    client: &MoatClient,
    action: TaskAction,
    json: bool,
    config: &Config,
) -> Result<(), ClientError> {
    match action {
        TaskAction::List {
            status,
            limit,
            offset,
        } => {
            let task_status = status.as_deref().and_then(parse_task_status);
            let query = ListTasksQuery {
                limit: Some(limit),
                offset,
                status: task_status,
                repository_id: None,
                created_by: None,
                tag: None,
                parent_task_id: None,
                followed_by: None,
            };
            let list = client.api().list_tasks(&query).await?;
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Get { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let task = client.api().get_task(&task_id).await?;
            if json {
                output::print_json_typed(&task);
            } else {
                format_task_detail(&task);
            }
        }
        TaskAction::Create {
            repo,
            workspace,
            same_sandbox,
            checkout_mode,
            name,
            parent,
            run_as,
            message,
            context_files,
            branch,
            agent_profile,
            model,
            headless,
        } => {
            // Agent-created tasks inside a sandbox always belong to the
            // ambient task, even if an explicit parent was also supplied.
            let parent_task_id = resolve_parent_task_id(config.task_id(), parent);

            let joined = match &same_sandbox {
                Some(task_ref) => {
                    Some(resolve_same_sandbox(client, config.task_id(), task_ref.as_deref()).await?)
                }
                None => None,
            };

            let repository_id = match &repo {
                Some(r) => Some(resolve_repo_id(client, r).await?),
                None => None,
            };
            // `--same-sandbox` conflicts with both, so at most one arm applies.
            let workspace_id = match (&workspace, &joined) {
                (Some(w), _) => Some(resolve_workspace_id(client, w).await?),
                (None, Some(join)) => join.workspace.clone(),
                (None, None) => None,
            };
            require_a_workspace_source(workspace_id.as_deref(), parent_task_id.as_deref())?;

            validate_context_files_have_message(&message, &context_files)?;
            let has_message = message.is_some();
            let file_ids = upload_context_files(client, &context_files).await?;
            let req = CreateTaskRequest {
                repository_id,
                workspace_id,
                sandbox_id: joined.as_ref().map(|j| j.sandbox.clone()),
                checkout_mode: checkout_mode.as_deref().map(|m| match m {
                    "isolated" => CheckoutMode::Isolated,
                    // The value_parser admits nothing else.
                    _ => CheckoutMode::Shared,
                }),
                name,
                message,
                branch,
                parent_task_id,
                run_as_user_id: run_as,
                // The CLI always drives the agent in edit mode; plan mode is a UI affordance.
                agent_mode: Some(AgentMode("edit".to_string())),
                agent_profile: Some(AgentProfile(agent_profile)),
                contexts: Vec::new(),
                description: None,
                file_ids,
                model,
                effort: None,
                headless: Some(headless),
                skills: Vec::new(),
            };
            let created = client.api().create_task(&req).await?;
            // A joined task skips provisioning entirely — saying "sandbox
            // provisioning" would be describing the thing --same-sandbox exists
            // to avoid.
            let sandbox_note = match &joined {
                Some(join) => format!("Joining task {}'s sandbox", join.task),
                None => "Sandbox provisioning".to_string(),
            };
            if json {
                output::print_json_typed(&created);
            } else if has_message {
                println!(
                    "Task {} created and started (status: {}). {sandbox_note}; agent will begin once the sandbox is ready.",
                    created.task_id, created.status
                );
            } else {
                println!(
                    "Task {} created (status: {}). {sandbox_note}; send a message with `moat send --task <id>` to start the agent.",
                    created.task_id, created.status
                );
            }
        }
        TaskAction::Fork {
            task_id,
            at_turn,
            message,
            same_sandbox,
            agent_profile,
        } => {
            let source_task_id = resolve_task_id(client, &task_id).await?;
            let has_message = message.is_some();
            let req = ForkTaskRequest {
                at_turn,
                message,
                same_sandbox: same_sandbox.then_some(true),
                effort: None,
                model: None,
                agent_profile,
                // The CLI always drives the agent in edit mode; plan mode is a
                // UI affordance.
                agent_mode: Some(AgentMode("edit".to_string())),
                skills: Vec::new(),
            };
            let created = client.api().fork_task(&source_task_id, &req).await?;
            if json {
                output::print_json_typed(&created);
                return Ok(());
            }
            let cut = match created.task.forked_from.as_ref().and_then(|f| f.at_turn) {
                Some(turn) => format!(" at turn {turn}"),
                None => String::new(),
            };
            println!(
                "Task {} forked from {source_task_id}{cut} (status: {}).",
                created.task_id, created.status
            );
            // A same-sandbox fork skips provisioning entirely, which is the
            // thing the flag exists to avoid saying.
            let sandbox_note = if same_sandbox {
                format!("Joining task {source_task_id}'s sandbox")
            } else {
                "Sandbox provisioning".to_string()
            };
            if has_message {
                println!("{sandbox_note}; agent will begin once the sandbox is ready.");
            } else {
                println!(
                    "{sandbox_note}; send a message with `moat send --task {}` to start the agent.",
                    created.task_id
                );
            }
        }
        TaskAction::Children { task_id } => {
            let parent = match task_id {
                Some(task_ref) => resolve_task_id(client, &task_ref).await?,
                None => config
                    .task_id()
                    .map(String::from)
                    .ok_or_else(|| ClientError::Api {
                        status: 400,
                        body: "No task_id provided and TASK_ID not set".to_string(),
                    })?,
            };
            let query = ListTasksQuery {
                limit: None,
                offset: None,
                status: None,
                repository_id: None,
                created_by: None,
                tag: None,
                parent_task_id: Some(parent),
                followed_by: None,
            };
            let list = client.api().list_tasks(&query).await?;
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Start { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().start_task(&task_id).await?;
            println!("Task {task_id} started.");
        }
        TaskAction::Stop { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().stop_task(&task_id).await?;
            println!("Task {task_id} stopped.");
        }
        TaskAction::Close {
            task_id,
            close_children,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client
                .api()
                .close_task(&task_id, &CloseTaskRequest { close_children })
                .await?;
            println!("Task {task_id} closed.");
        }
        TaskAction::Reopen { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().reopen_task(&task_id).await?;
            println!("Task {task_id} reopened.");
        }
        TaskAction::Update {
            task_id,
            name,
            description,
            model,
            agent_profile,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            if name.is_none() && description.is_none() && model.is_none() && agent_profile.is_none()
            {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Provide --name, --description, --model and/or --agent-profile"
                        .to_string(),
                });
            }
            let req = UpdateTaskMetadataRequest {
                name,
                description,
                model,
                agent_profile,
            };
            let updated = client.api().update_task_metadata(&task_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Task {} updated.", updated.task_id);
            }
        }
        TaskAction::Ttl {
            task_id,
            ttl_minutes,
            inactive_ttl_days,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            if ttl_minutes.is_none() && inactive_ttl_days.is_none() {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Provide --ttl-minutes and/or --inactive-ttl-days".to_string(),
                });
            }
            let req = UpdateTaskTtlRequest {
                ttl_minutes,
                inactive_ttl_days,
            };
            let updated = client.api().update_ttl(&task_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!(
                    "Task {} TTL updated (idle={}m, inactive={}d).",
                    updated.task_id, updated.ttl_minutes, updated.inactive_ttl_days
                );
            }
        }
        TaskAction::Status { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let status = client.api().sandbox_status(&task_id).await?;
            if json {
                output::print_json_typed(&status);
            } else {
                format_sandbox_status(&status);
            }
        }
        TaskAction::Restart { task_id, yes } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let note = self_restart_note(config, &task_id, yes, SandboxOperation::Restart)?;
            record_wake_note(&config.sandbox_grpc_port, &task_id, note.as_deref()).await?;
            match client.api().restart_sandbox(&task_id).await {
                Ok(()) => println!("Sandbox for {task_id} restarted."),
                Err(e) => {
                    let e = ClientError::from(e);
                    cancel_wake_note(&config.sandbox_grpc_port, &task_id, note.is_some(), &e).await;
                    return Err(e);
                }
            }
        }
        TaskAction::Redeploy { task_id, yes } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let note = self_restart_note(config, &task_id, yes, SandboxOperation::Redeploy)?;
            record_wake_note(&config.sandbox_grpc_port, &task_id, note.as_deref()).await?;
            match client.api().redeploy_sandbox(&task_id).await {
                Ok(resp) => println!(
                    "Sandbox for {task_id} redeployed (status: {:?}).",
                    resp.status
                ),
                Err(e) => {
                    let e = ClientError::from(e);
                    cancel_wake_note(&config.sandbox_grpc_port, &task_id, note.is_some(), &e).await;
                    return Err(e);
                }
            }
        }
        TaskAction::Logs {
            task_id,
            previous,
            no_follow,
            tail,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            stream_logs(client, &task_id, previous, !no_follow, tail).await?;
        }
        TaskAction::Diagnose { task_id, limit } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_diagnose(client, &task_id, limit, json).await?;
        }
        TaskAction::WorkspaceRepo { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let access = client.api().workspace_repository_access(&task_id).await?;
            if json {
                output::print_json_typed(&access);
            } else {
                println!("export AWS_ACCESS_KEY_ID={}", access.access_key_id);
                println!("export AWS_SECRET_ACCESS_KEY={}", access.secret_access_key);
                println!("export AWS_SESSION_TOKEN={}", access.session_token);
                println!("export AWS_DEFAULT_REGION={}", access.region);
                println!("export RESTIC_REPOSITORY={}", access.repository);
                println!("export RESTIC_PASSWORD={}", access.password);
                println!();
                println!("# credentials expire {}", access.expires_at);
                println!("# then: restic snapshots");
            }
        }
        TaskAction::Files { task_id, action } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_file_action(client, &task_id, action, json).await?;
        }
        TaskAction::Scripts { task_id, action } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_task_script_action(client, &task_id, action, json).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TaskAction` is a `Subcommand`, so parsing it standalone needs a
    /// `Parser` to hang it off.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        action: TaskAction,
    }

    fn parse_create(extra: &[&str]) -> Result<TaskAction, clap::Error> {
        let mut argv = vec!["moat", "create", "--agent-profile", "claude-code"];
        argv.extend_from_slice(extra);
        <TestCli as clap::Parser>::try_parse_from(argv).map(|cli| cli.action)
    }

    /// `(flag present, explicit task id)` — flattened rather than handed back
    /// as the `Option<Option<_>>` clap parses it into, which trips
    /// `clippy::option_option`.
    fn same_sandbox_of(action: &TaskAction) -> (bool, Option<&str>) {
        match action {
            TaskAction::Create { same_sandbox, .. } => (
                same_sandbox.is_some(),
                same_sandbox.as_ref().and_then(|v| v.as_deref()),
            ),
            _ => unreachable!("parse_create only builds Create"),
        }
    }

    #[test]
    fn update_takes_an_agent_profile_with_or_without_a_model() {
        let argv = ["moat", "update", "t-1", "--agent-profile", "codex"];
        let Ok(TaskAction::Update {
            agent_profile,
            model,
            ..
        }) = <TestCli as clap::Parser>::try_parse_from(argv).map(|cli| cli.action)
        else {
            panic!("update parses");
        };
        assert_eq!(agent_profile.as_deref(), Some("codex"));
        assert_eq!(model, None);
    }

    #[test]
    fn bare_same_sandbox_parses_as_present_without_a_task_id() {
        // The distinction the flag turns on: absent (no join) vs present with
        // no value (join the ambient task) vs present with an explicit target.
        let absent = parse_create(&["--workspace", "ws_1"]).expect("no join");
        assert_eq!(same_sandbox_of(&absent), (false, None));

        let ambient = parse_create(&["--same-sandbox"]).expect("bare join");
        assert_eq!(same_sandbox_of(&ambient), (true, None));

        let explicit = parse_create(&["--same-sandbox", "abc-123"]).expect("explicit join");
        assert_eq!(same_sandbox_of(&explicit), (true, Some("abc-123")));
    }

    #[test]
    fn same_sandbox_conflicts_with_naming_a_repo_or_workspace() {
        // The joined Sandbox already fixes both, so either one could only agree
        // redundantly or contradict — better rejected here than server-side.
        assert!(parse_create(&["--same-sandbox", "--repo", "moatless"]).is_err());
        assert!(parse_create(&["--same-sandbox", "--workspace", "ws_1"]).is_err());
    }

    #[test]
    fn checkout_mode_requires_a_join_and_admits_only_the_two_modes() {
        assert!(
            parse_create(&["--workspace", "ws_1", "--checkout-mode", "isolated"]).is_err(),
            "checkout mode is meaningless without a sandbox to be isolated from"
        );
        parse_create(&["--same-sandbox", "--checkout-mode", "isolated"]).expect("isolated");
        parse_create(&["--same-sandbox", "--checkout-mode", "shared"]).expect("shared");
        assert!(parse_create(&["--same-sandbox", "--checkout-mode", "private"]).is_err());
    }
}

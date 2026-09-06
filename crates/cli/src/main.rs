#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `moat` — CLI for the Moatless backend API.
//!
//! Two modes selected at startup by the `USER_AUTH_TOKEN` env var:
//! - **Sandbox** (inside a pod): no login/logout, adds `set-name` / `set-tags`
//! - **Standalone** (human user): login/logout, no sandbox-only commands

use clap::{Parser, Subcommand};
use moatless_api_schemas::task::{SetTagsRequest, UpdateTaskMetadataRequest};

use moatless_cli::auth;
use moatless_cli::client::MoatClient;
use moatless_cli::commands;
use moatless_cli::config::Config;
use moatless_cli::credentials;
use moatless_cli::output;

// ---------------------------------------------------------------------------
// Global options (shared between both CLI modes)
// ---------------------------------------------------------------------------

/// What this binary was built from, for `moat --version`.
///
/// `GITHUB_SHA` is set for every CI build and absent locally, where the working
/// tree is the answer. Dates a `moat` against the backend it answers to — see
/// [`moatless_cli::client::ClientError::UndecodableResponse`] for the skew.
fn version() -> String {
    let commit =
        option_env!("GITHUB_SHA").map_or("local build", |sha| sha.get(..12).unwrap_or(sha));
    format!("{} ({commit})", env!("CARGO_PKG_VERSION"))
}

#[derive(Parser, Clone)]
struct GlobalOpts {
    /// Output raw JSON instead of human-readable tables
    #[arg(long, global = true)]
    json: bool,

    /// Show HTTP request details for debugging
    #[arg(long, global = true)]
    verbose: bool,

    /// Seconds to wait for a response, overriding the 30-second default.
    /// Raise it for commands whose endpoint provisions or removes a sandbox
    /// before it answers.
    #[arg(long, global = true)]
    timeout: Option<u64>,
}

// ---------------------------------------------------------------------------
// Commands available in BOTH modes
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum SharedCommands {
    /// Show current user info
    Whoami,
    /// Show current context (API URL, auth mode, profile)
    Status,
    /// Task management
    Tasks {
        #[command(subcommand)]
        action: commands::tasks::TaskAction,
    },
    /// Agent harness and model catalog
    Agents {
        #[command(subcommand)]
        action: commands::agents::AgentAction,
    },
    /// Repository management
    Repos {
        #[command(subcommand)]
        action: Box<commands::repos::RepoAction>,
    },
    /// Workspace management
    Workspaces {
        #[command(subcommand)]
        action: commands::workspaces::WorkspaceAction,
    },
    /// Adapter connection management
    Adapters {
        #[command(subcommand)]
        action: commands::adapters::AdaptersAction,
    },
    /// Linear issue and comment operations via the workspace OAuth token
    Linear {
        #[command(subcommand)]
        action: Box<commands::linear::LinearAction>,
    },
    /// GitHub operations — `pr create`, `pr comment`, and `issue comment` are
    /// proxied through the backend (typed task binding + webhook dedup);
    /// everything else passes through to the local `gh` CLI verbatim.
    Gh {
        /// Arguments as you would pass them to `gh` (e.g. `pr create --title …`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Loop management (recurring agent work fired by subscriptions or schedules)
    Loops {
        #[command(subcommand)]
        action: Box<commands::loops::LoopsAction>,
    },
    /// Task binding management
    Bindings {
        #[command(subcommand)]
        action: Box<commands::adapters::BindingAction>,
    },
    /// Make a raw API request and report the status alongside the body
    Api(commands::api::ApiArgs),
    /// Task event history
    Events {
        #[command(subcommand)]
        action: commands::events::EventAction,
    },
    /// Team management
    Teams {
        #[command(subcommand)]
        action: commands::teams::TeamAction,
    },
    /// Plugin management
    Plugins {
        #[command(subcommand)]
        action: commands::plugins::PluginAction,
    },
    /// Provider settings (GitHub PAT, etc.)
    Settings {
        #[command(subcommand)]
        action: commands::settings::SettingsAction,
    },
    /// Secret management
    #[command(name = "secret")]
    Secrets {
        #[command(subcommand)]
        action: commands::secrets::SecretAction,
    },
    /// List tags
    Tags,
    /// Kubernetes installation workflow (preflight, init, secrets, install, status)
    K8s {
        #[command(subcommand)]
        action: commands::k8s::K8sAction,
    },
    /// Docker Compose installation workflow (preflight, init, install, status, logs)
    Docker {
        #[command(subcommand)]
        action: commands::docker::DockerAction,
    },
    /// Manage users and their provider identity mappings
    User {
        /// Moatless username (login)
        login: String,
        #[command(subcommand)]
        action: commands::users::UserAction,
    },
    /// Bot user management — provision users backed by a registered GitHub App.
    Bot {
        #[command(subcommand)]
        action: commands::bots::BotAction,
    },
    /// GitHub App administration — register, rotate a signing key, remove.
    #[command(name = "github-app")]
    GithubApp {
        #[command(subcommand)]
        action: commands::github_apps::GitHubAppAction,
    },
    /// Send a message — drive a task, DM a user, or pass through an adapter
    /// destination (Slack channel/DM/thread, Linear issue, ...). Replies and
    /// emoji reactions to specific messages live under `moat messages`.
    Send(Box<commands::send::SendArgs>),
    /// Message management — list, reply, react.
    Messages {
        #[command(subcommand)]
        action: commands::messages::MessageAction,
    },
    /// Record feedback about a task to be used for improving moatless logic,
    /// agent skills, or tooling — or, with `list`, read what has been recorded
    Feedback {
        #[command(subcommand)]
        action: Option<commands::feedback::FeedbackAction>,
        /// The feedback message (what went wrong / what could be better).
        #[arg(long, short = 'c')]
        comment: Option<String>,
        /// Task the feedback is about. Defaults to `$TASK_ID`.
        #[arg(long)]
        task: Option<String>,
        /// What the feedback is about: `environment` (the sandbox),
        /// `codebase` (this repository's code or tooling), `instruction`
        /// (`AGENTS.md`, `CLAUDE.md`, or a skill), or `other`.
        #[arg(long)]
        category: Option<String>,
        /// Optional: scope feedback to a specific assistant message
        /// (`task_messages.uuid`). Left unset, it best-effort attaches to the
        /// task's most recent activity instead of the whole task.
        #[arg(long)]
        message: Option<String>,
        /// Optional: scope feedback to a specific tool call within the
        /// referenced message. Requires `--message`.
        #[arg(long = "tool-use-id")]
        tool_use_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Sandbox mode CLI (inside a pod — no login/logout, adds task shortcuts)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless sandbox CLI", version = version())]
struct SandboxCli {
    #[command(subcommand)]
    command: SandboxCommands,

    #[command(flatten)]
    opts: GlobalOpts,
}

#[derive(Subcommand)]
enum SandboxCommands {
    #[command(flatten)]
    Shared(Box<SharedCommands>),

    /// Set the name of the current task
    SetName {
        /// The new task name
        name: String,
    },

    /// Set tags on the current task (replaces existing tags)
    SetTags {
        /// Tag names (space-separated)
        tags: Vec<String>,
    },

    /// Run a command that outlives one Bash tool call. The sandbox owns the
    /// process, so it stays awake until the command finishes and wakes you
    /// when it does.
    Cmd {
        #[command(subcommand)]
        action: commands::cmd::CmdAction,
    },

}

// ---------------------------------------------------------------------------
// Standalone mode CLI (human user — has login/logout)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless CLI", version = version())]
struct StandaloneCli {
    #[command(subcommand)]
    command: StandaloneCommands,

    #[command(flatten)]
    opts: GlobalOpts,
}

#[derive(Subcommand)]
enum StandaloneCommands {
    #[command(flatten)]
    Shared(Box<SharedCommands>),

    /// Authenticate with the Moatless API.
    ///
    /// Asks the server which flow it runs (`GET /api/v1/auth/mode`):
    /// - Local password auth → prompts for a username and password. Honors
    ///   `MOATLESS_EMAIL` / `MOATLESS_PASSWORD` for non-interactive use.
    ///   `MOATLESS_USERNAME` remains accepted as a compatibility alias.
    /// - An OAuth provider → runs the browser-based flow.
    Login {
        /// URL to authenticate against. Defaults to the configured API URL
        /// (env `MOATLESS_API_URL` or the default profile).
        #[arg(long)]
        url: Option<String>,
    },
    /// Remove saved credentials
    Logout,
}

// ---------------------------------------------------------------------------
// Entry point — select CLI mode before parsing
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let is_sandbox = moatless_cli::config::sandbox_token().is_some();

    let result = if is_sandbox {
        let cli = SandboxCli::parse();
        run_sandbox(cli).await
    } else {
        let cli = StandaloneCli::parse();
        run_standalone(cli).await
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Sandbox dispatch
// ---------------------------------------------------------------------------

async fn run_sandbox(cli: SandboxCli) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env();
    let mut moat = MoatClient::new(&cfg, cli.opts.verbose);
    if let Some(seconds) = cli.opts.timeout {
        moat.set_http_timeout(std::time::Duration::from_secs(seconds));
    }

    let needs_auth = !runs_without_auth_sandbox(&cli.command);
    if needs_auth {
        auth::ensure_auth(&mut moat, &cfg).await?;
    }

    let result = dispatch_sandbox(cli.command, &moat, &cfg, cli.opts.json).await;

    // A cached sandbox session can be stale (expired or rotated). On rejection,
    // drop it so the next `moat` invocation re-exchanges the refreshed user
    // token instead of replaying the dead session into another 401.
    if needs_auth && is_unauthorized(&result) {
        auth::invalidate_session(&cfg);
    }

    result
}

/// Whether a command result failed because the request was rejected as
/// unauthorized. Commands surface a 401 either as a `ClientError` (raw helpers)
/// or an `ApiError` (the typed client), so check both boxed error types.
fn is_unauthorized(result: &Result<(), Box<dyn std::error::Error>>) -> bool {
    let Some(err) = result.as_ref().err() else {
        return false;
    };
    matches!(
        err.downcast_ref::<moatless_cli::client::ClientError>(),
        Some(moatless_cli::client::ClientError::Unauthorized)
    ) || matches!(
        err.downcast_ref::<moatless_api_client::ApiError>(),
        Some(moatless_api_client::ApiError::Unauthorized)
    )
}

async fn dispatch_sandbox(
    command: SandboxCommands,
    moat: &MoatClient,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        SandboxCommands::Shared(cmd) => run_shared(*cmd, moat, cfg, json).await?,
        SandboxCommands::SetName { name } => {
            let task_id = require_task_id(cfg)?;
            let req = UpdateTaskMetadataRequest {
                name: Some(name.clone()),
                description: None,
            };
            let task = moat.api().update_task_metadata(&task_id, &req).await?;
            if json {
                output::print_json_typed(&task);
            } else {
                println!("Task name set to: {name}");
            }
        }
        SandboxCommands::SetTags { tags } => {
            let task_id = require_task_id(cfg)?;
            let req = SetTagsRequest { tags: tags.clone() };
            moat.api().set_task_tags(&task_id, &req).await?;
            if json {
                output::print_json_typed(&tags);
            } else {
                println!("Tags set: {}", tags.join(", "));
            }
        }
        SandboxCommands::Cmd { action } => {
            let task_id = require_task_id(cfg)?;
            commands::cmd::run(action, task_id, json).await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Standalone dispatch
// ---------------------------------------------------------------------------

async fn run_standalone(cli: StandaloneCli) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env();
    let mut moat = MoatClient::new(&cfg, cli.opts.verbose);
    if let Some(seconds) = cli.opts.timeout {
        moat.set_http_timeout(std::time::Duration::from_secs(seconds));
    }

    let needs_auth = !matches!(
        &cli.command,
        StandaloneCommands::Login { .. } | StandaloneCommands::Logout
    ) && !is_status_command_standalone(&cli.command)
        && !is_raw_api_command(&cli.command);
    if needs_auth {
        auth::ensure_auth(&mut moat, &cfg).await?;
    }

    match cli.command {
        StandaloneCommands::Shared(cmd) => run_shared(*cmd, &moat, &cfg, cli.opts.json).await?,
        StandaloneCommands::Login { url } => {
            auth::login(&cfg, url).await?;
        }
        StandaloneCommands::Logout => {
            let host = cfg.host();
            match credentials::delete_credential(&host) {
                Ok(true) => println!("Logged out from {host}."),
                Ok(false) => println!("No credentials found for {host}."),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared command handler (called by both modes)
// ---------------------------------------------------------------------------

async fn run_shared(
    cmd: SharedCommands,
    moat: &MoatClient,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        SharedCommands::Whoami => {
            let session = moat.api().me().await?;
            if json {
                output::print_json_typed(&session);
            } else {
                commands::info::format_whoami(&session);
            }
        }
        SharedCommands::Status => {
            commands::info::print_status(cfg);
        }
        SharedCommands::Tasks { action } => {
            commands::tasks::run(moat, action, json, cfg).await?;
        }
        SharedCommands::Agents { action } => {
            commands::agents::run(moat, action, json).await?;
        }
        SharedCommands::Repos { action } => {
            commands::repos::run(moat, *action, json).await?;
        }
        SharedCommands::Workspaces { action } => {
            commands::workspaces::run(moat, action, json).await?;
        }
        SharedCommands::Adapters { action } => {
            commands::adapters::run_adapters(moat, action, json).await?;
        }
        SharedCommands::Linear { action } => {
            commands::linear::run(moat, cfg, *action, json).await?;
        }
        SharedCommands::Gh { args } => {
            commands::gh::run(moat, cfg, args, json).await?;
        }
        SharedCommands::Loops { action } => {
            commands::loops::run_loops(moat, *action, json).await?;
        }
        SharedCommands::Bindings { action } => {
            commands::adapters::run_bindings(moat, *action, json).await?;
        }
        SharedCommands::Api(args) => {
            commands::api::run(moat, args).await?;
        }
        SharedCommands::Events { action } => {
            commands::events::run(moat, action, json).await?;
        }
        SharedCommands::Teams { action } => {
            commands::teams::run(moat, action, json).await?;
        }
        SharedCommands::Settings { action } => {
            commands::settings::run(moat, action, json).await?;
        }
        SharedCommands::Secrets { action } => {
            commands::secrets::run(moat, action, json).await?;
        }
        SharedCommands::Plugins { action } => {
            commands::plugins::run(moat, action, json).await?;
        }
        SharedCommands::Tags => {
            let tags = moat.api().list_tags().await?;
            if json {
                output::print_json_typed(&tags);
            } else {
                let rows: Vec<Vec<String>> = tags
                    .iter()
                    .map(|t| vec![t.tag_id.clone(), t.name.clone()])
                    .collect();
                output::print_table(&["TAG_ID", "NAME"], &rows);
            }
        }
        SharedCommands::K8s { action } => {
            commands::k8s::run(action, json).await?;
        }
        SharedCommands::Docker { action } => {
            commands::docker::run(action, json).await?;
        }
        SharedCommands::User { login, action } => {
            commands::users::run(moat, &login, action, json).await?;
        }
        SharedCommands::Bot { action } => {
            commands::bots::run(moat, action, json).await?;
        }
        SharedCommands::GithubApp { action } => {
            commands::github_apps::run(moat, action, json).await?;
        }
        SharedCommands::Send(args) => {
            commands::send::run(moat, *args, cfg, json).await?;
        }
        SharedCommands::Messages { action } => {
            commands::messages::run(moat, action, json).await?;
        }
        SharedCommands::Feedback {
            action,
            comment,
            task,
            category,
            message,
            tool_use_id,
        } => match action {
            Some(commands::feedback::FeedbackAction::List {
                workspace,
                repo,
                task,
                source,
                surface,
                category,
                status,
                created_by,
                since,
                limit,
                offset,
                full,
            }) => {
                commands::feedback::list(
                    moat, workspace, repo, task, source, surface, category, status, created_by,
                    since, limit, offset, full, json,
                )
                .await?;
            }
            Some(commands::feedback::FeedbackAction::Status {
                feedback_id,
                status,
                note,
            }) => {
                commands::feedback::set_status(moat, feedback_id, status, note, json).await?;
            }
            None => {
                let comment = comment
                    .ok_or("--comment is required to record feedback (or use `feedback list`)")?;
                if tool_use_id.is_some() && message.is_none() {
                    return Err("--tool-use-id requires --message".into());
                }
                let task_id = match task {
                    Some(id) => id,
                    None => require_task_id(cfg)?,
                };
                commands::feedback::run(
                    moat,
                    commands::feedback::FeedbackArgs {
                        task_id,
                        comment,
                        category,
                        message_uuid: message,
                        tool_use_id,
                    },
                    json,
                )
                .await?;
            }
        },
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_task_id(cfg: &Config) -> Result<String, Box<dyn std::error::Error>> {
    cfg.task_id()
        .map(String::from)
        .ok_or_else(|| "TASK_ID environment variable is required for this command".into())
}

/// `moat cmd` talks only to the sandbox's own loopback gRPC server, so it must
/// keep working when the backend session is stale — that is exactly when a build
/// still needs supervising.
fn runs_without_auth_sandbox(cmd: &SandboxCommands) -> bool {
    match cmd {
        SandboxCommands::Cmd { .. } => true,
        SandboxCommands::Shared(c) => matches!(
            **c,
            SharedCommands::Status | SharedCommands::K8s { .. } | SharedCommands::Docker { .. }
        ),
        _ => false,
    }
}

fn is_status_command_standalone(cmd: &StandaloneCommands) -> bool {
    matches!(
        cmd,
        StandaloneCommands::Shared(c)
            if matches!(
                **c,
                SharedCommands::Status
                    | SharedCommands::K8s { .. }
                    | SharedCommands::Docker { .. }
            )
    )
}

/// `moat api` reports whatever the server answers, including a rejection, so it
/// must not be stopped by the pre-flight auth check — being turned away is a
/// result it is expected to print.
fn is_raw_api_command(cmd: &StandaloneCommands) -> bool {
    matches!(
        cmd,
        StandaloneCommands::Shared(c) if matches!(**c, SharedCommands::Api(_))
    )
}

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

#[derive(Parser, Clone)]
struct GlobalOpts {
    /// Output raw JSON instead of human-readable tables
    #[arg(long, global = true)]
    json: bool,

    /// Show HTTP request details for debugging
    #[arg(long, global = true)]
    verbose: bool,
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
    /// Repository management
    Repos {
        #[command(subcommand)]
        action: Box<commands::repos::RepoAction>,
    },
    /// Adapter connection management
    Adapters {
        #[command(subcommand)]
        action: commands::adapters::AdaptersAction,
    },
    /// Trigger management
    Triggers {
        #[command(subcommand)]
        action: Box<commands::adapters::TriggerAction>,
    },
    /// Task binding management
    Bindings {
        #[command(subcommand)]
        action: Box<commands::adapters::BindingAction>,
    },
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
    /// Plugin repo management
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
    /// Command management
    #[command(name = "commands")]
    Commands {
        #[command(subcommand)]
        action: commands::command_mgmt::CommandAction,
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
    /// Send a message — drive a task, DM a user, or pass through an adapter
    /// destination (Slack channel/DM/thread, Linear issue, ...). Replies and
    /// emoji reactions to specific messages live under `moat messages`.
    Send(Box<commands::send::SendArgs>),
    /// Message management — list, reply, react.
    Messages {
        #[command(subcommand)]
        action: commands::messages::MessageAction,
    },
    /// Task template management
    Templates {
        #[command(subcommand)]
        action: Box<commands::templates::TemplateAction>,
    },
}

// ---------------------------------------------------------------------------
// Sandbox mode CLI (inside a pod — no login/logout, adds task shortcuts)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless sandbox CLI")]
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

    /// Record feedback about this task to be used for improving moatless
    /// logic, agent skills, or tooling.
    Feedback {
        /// The feedback message (what went wrong / what could be better).
        #[arg(long, short = 'c')]
        comment: String,
        /// Optional: scope feedback to a specific assistant message
        /// (`task_messages.uuid`). Defaults to task-level feedback.
        #[arg(long)]
        message: Option<String>,
        /// Optional: scope feedback to a specific tool call within the
        /// referenced message. Requires `--message`.
        #[arg(long = "tool-use-id")]
        tool_use_id: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Standalone mode CLI (human user — has login/logout)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "moat", about = "Moatless CLI")]
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
    /// Probes the target URL's auth mode and picks the right flow:
    /// - No providers configured (e.g. local Docker install) → saves a
    ///   `local-user` profile, no browser needed.
    /// - Providers configured → runs the browser-based OAuth flow.
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
    let is_sandbox = std::env::var("USER_AUTH_TOKEN").is_ok();

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

    let needs_auth = !is_status_command_sandbox(&cli.command);
    if needs_auth {
        auth::ensure_auth(&mut moat, &cfg).await?;
    }

    match cli.command {
        SandboxCommands::Shared(cmd) => run_shared(*cmd, &moat, &cfg, cli.opts.json).await?,
        SandboxCommands::SetName { name } => {
            let task_id = require_task_id(&cfg)?;
            let req = UpdateTaskMetadataRequest {
                name: Some(name.clone()),
                description: None,
            };
            let task = moat.api().update_task_metadata(&task_id, &req).await?;
            if cli.opts.json {
                output::print_json_typed(&task);
            } else {
                println!("Task name set to: {name}");
            }
        }
        SandboxCommands::SetTags { tags } => {
            let task_id = require_task_id(&cfg)?;
            let req = SetTagsRequest { tags: tags.clone() };
            moat.api().set_task_tags(&task_id, &req).await?;
            if cli.opts.json {
                output::print_json_typed(&tags);
            } else {
                println!("Tags set: {}", tags.join(", "));
            }
        }
        SandboxCommands::Feedback {
            comment,
            message,
            tool_use_id,
        } => {
            let task_id = require_task_id(&cfg)?;
            if tool_use_id.is_some() && message.is_none() {
                return Err("--tool-use-id requires --message".into());
            }
            commands::feedback::run(
                &moat,
                commands::feedback::FeedbackArgs {
                    task_id,
                    comment,
                    message_uuid: message,
                    tool_use_id,
                },
                cli.opts.json,
            )
            .await?;
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

    let needs_auth = !matches!(
        &cli.command,
        StandaloneCommands::Login { .. } | StandaloneCommands::Logout
    ) && !is_status_command_standalone(&cli.command);
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
        SharedCommands::Repos { action } => {
            commands::repos::run(moat, *action, json).await?;
        }
        SharedCommands::Adapters { action } => {
            commands::adapters::run_adapters(moat, action, json).await?;
        }
        SharedCommands::Triggers { action } => {
            commands::adapters::run_triggers(moat, *action, json).await?;
        }
        SharedCommands::Bindings { action } => {
            commands::adapters::run_bindings(moat, *action, json).await?;
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
        SharedCommands::Commands { action } => {
            commands::command_mgmt::run(moat, action, json).await?;
        }
        SharedCommands::Tags => {
            let tags = moat.api().list_tags().await?;
            if json {
                output::print_json_typed(&tags);
            } else {
                commands::misc::format_tags(&tags);
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
        SharedCommands::Templates { action } => {
            commands::templates::run(moat, *action, json).await?;
        }
        SharedCommands::Send(args) => {
            commands::send::run(moat, *args, cfg, json).await?;
        }
        SharedCommands::Messages { action } => {
            commands::messages::run(moat, action, json).await?;
        }
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

fn is_status_command_sandbox(cmd: &SandboxCommands) -> bool {
    matches!(
        cmd,
        SandboxCommands::Shared(c)
            if matches!(
                **c,
                SharedCommands::Status
                    | SharedCommands::K8s { .. }
                    | SharedCommands::Docker { .. }
            )
    )
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


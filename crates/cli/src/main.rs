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
        action: commands::repos::RepoAction,
    },
    /// Message management
    Messages {
        #[command(subcommand)]
        action: commands::messages::MessageAction,
    },
    /// Channel management
    Channels {
        #[command(subcommand)]
        action: commands::channels::ChannelAction,
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
    /// List available commands
    #[command(name = "commands")]
    Cmds,
    /// List tags
    Tags,
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

    /// Authenticate with the Moatless API
    Login,
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
        StandaloneCommands::Login | StandaloneCommands::Logout
    ) && !is_status_command_standalone(&cli.command);
    if needs_auth {
        auth::ensure_auth(&mut moat, &cfg).await?;
    }

    match cli.command {
        StandaloneCommands::Shared(cmd) => run_shared(*cmd, &moat, &cfg, cli.opts.json).await?,
        StandaloneCommands::Login => {
            auth::login_browser(&cfg).await?;
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
            commands::repos::run(moat, action, json).await?;
        }
        SharedCommands::Messages { action } => {
            commands::messages::run(moat, action, json).await?;
        }
        SharedCommands::Channels { action } => {
            commands::channels::run(moat, action, json).await?;
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
        SharedCommands::Plugins { action } => {
            commands::plugins::run(moat, action, json).await?;
        }
        SharedCommands::Cmds => {
            let cmds = moat.api().list_commands().await?;
            if json {
                output::print_json_typed(&cmds);
            } else {
                commands::misc::format_commands(&cmds);
            }
        }
        SharedCommands::Tags => {
            let tags = moat.api().list_tags().await?;
            if json {
                output::print_json_typed(&tags);
            } else {
                commands::misc::format_tags(&tags);
            }
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
    matches!(cmd, SandboxCommands::Shared(c) if matches!(**c, SharedCommands::Status))
}

fn is_status_command_standalone(cmd: &StandaloneCommands) -> bool {
    matches!(cmd, StandaloneCommands::Shared(c) if matches!(**c, SharedCommands::Status))
}

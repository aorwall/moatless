#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `moat` — Sandbox CLI for the Moatless backend API.

use clap::{Parser, Subcommand};

use moatless_cli::auth;
use moatless_cli::client::MoatClient;
use moatless_cli::commands;
use moatless_cli::config::Config;
use moatless_cli::credentials;
use moatless_cli::output;

#[derive(Parser)]
#[command(name = "moat", about = "Moatless sandbox CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output raw JSON instead of human-readable tables
    #[arg(long, global = true)]
    json: bool,

    /// Show HTTP request details for debugging
    #[arg(long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with the Moatless API
    Login,
    /// Remove saved credentials
    Logout,
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env();
    let mut moat = MoatClient::new(&cfg, cli.verbose);

    // For commands that require auth (everything except Login, Logout, Status)
    let needs_auth = !matches!(
        cli.command,
        Commands::Login | Commands::Logout | Commands::Status
    );
    if needs_auth {
        auth::ensure_auth(&mut moat, &cfg).await?;
    }

    match cli.command {
        Commands::Login => {
            if cfg.is_sandbox() {
                auth::login_sandbox(&mut moat, &cfg).await?;
                println!("Login successful.");
            } else {
                auth::login_browser(&cfg).await?;
            }
        }
        Commands::Logout => {
            if cfg.is_sandbox() {
                // In sandbox mode, delete session file
                let _ = std::fs::remove_file(&cfg.session_file);
                println!("Session cleared.");
            } else {
                let host = cfg.host();
                match credentials::delete_credential(&host) {
                    Ok(true) => println!("Logged out from {host}."),
                    Ok(false) => println!("No credentials found for {host}."),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
        }
        Commands::Whoami => {
            let session = moat.api().me().await?;
            if cli.json {
                output::print_json_typed(&session);
            } else {
                commands::info::format_whoami(&session);
            }
        }
        Commands::Status => {
            commands::info::print_status(&cfg);
        }
        Commands::Tasks { action } => {
            commands::tasks::run(&moat, action, cli.json, &cfg).await?;
        }
        Commands::Repos { action } => {
            commands::repos::run(&moat, action, cli.json).await?;
        }
        Commands::Messages { action } => {
            commands::messages::run(&moat, action, cli.json).await?;
        }
        Commands::Channels { action } => {
            commands::channels::run(&moat, action, cli.json).await?;
        }
        Commands::Events { action } => {
            commands::events::run(&moat, action, cli.json).await?;
        }
        Commands::Teams { action } => {
            commands::teams::run(&moat, action, cli.json).await?;
        }
        Commands::Settings { action } => {
            commands::settings::run(&moat, action, cli.json).await?;
        }
        Commands::Plugins { action } => {
            commands::plugins::run(&moat, action, cli.json).await?;
        }
        Commands::Cmds => {
            let cmds = moat.api().list_commands().await?;
            if cli.json {
                output::print_json_typed(&cmds);
            } else {
                commands::misc::format_commands(&cmds);
            }
        }
        Commands::Tags => {
            let tags = moat.api().list_tags().await?;
            if cli.json {
                output::print_json_typed(&tags);
            } else {
                commands::misc::format_tags(&tags);
            }
        }
    }

    Ok(())
}

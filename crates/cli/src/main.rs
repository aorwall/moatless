#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `moat` — Sandbox CLI for the Moatless backend API.

mod auth;
mod client;
mod commands;
mod config;
pub mod credentials;
mod output;

use clap::{Parser, Subcommand};

use crate::client::map_api_error;

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
    /// Sandbox environment management
    Sandbox {
        #[command(subcommand)]
        action: commands::sandbox::SandboxAction,
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
    /// Team management
    Teams {
        #[command(subcommand)]
        action: commands::teams::TeamAction,
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
    let cfg = config::Config::from_env();
    let mut moat = client::MoatClient::new(&cfg, cli.verbose);

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
            let resp = moat
                .api()
                .me_handler()
                .await
                .map_err(map_api_error)?;
            let session = resp.into_inner();
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
        Commands::Sandbox { action } => {
            commands::sandbox::run(&moat, action, cli.json).await?;
        }
        Commands::Messages { action } => {
            commands::messages::run(&moat, action, cli.json).await?;
        }
        Commands::Channels { action } => {
            commands::channels::run(&moat, action, cli.json).await?;
        }
        Commands::Teams { action } => {
            commands::teams::run(&moat, action, cli.json).await?;
        }
        Commands::Cmds => {
            let resp = moat
                .api()
                .list_commands()
                .await
                .map_err(map_api_error)?;
            let cmds = resp.into_inner();
            if cli.json {
                output::print_json_typed(&cmds);
            } else {
                commands::misc::format_commands(&cmds);
            }
        }
        Commands::Tags => {
            let resp = moat
                .api()
                .list_tags()
                .await
                .map_err(map_api_error)?;
            let tags = resp.into_inner();
            if cli.json {
                output::print_json_typed(&tags);
            } else {
                commands::misc::format_tags(&tags);
            }
        }
    }

    Ok(())
}

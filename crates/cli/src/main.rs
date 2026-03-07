#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `moat` — Sandbox CLI for the Moatless backend API.

mod auth;
mod client;
mod commands;
mod config;
mod output;

use clap::{Parser, Subcommand};

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
    /// Exchange token for session (auto on first use)
    Login,
    /// Show current user info
    Whoami,
    /// Show current sandbox context
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
    let cfg = config::Config::from_env()?;
    let mut moat = client::MoatClient::new(&cfg, cli.verbose);

    // Ensure we have a session (auto-login)
    if !matches!(cli.command, Commands::Login) {
        auth::ensure_session(&mut moat, &cfg).await?;
    }

    match cli.command {
        Commands::Login => {
            auth::login(&mut moat, &cfg).await?;
            println!("Login successful.");
        }
        Commands::Whoami => {
            let resp = moat.get("/api/v1/auth/me").await?;
            if cli.json {
                output::print_json(&resp);
            } else {
                commands::info::format_whoami(&resp);
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
        Commands::Cmds => {
            let resp = moat.get("/api/v1/commands").await?;
            if cli.json {
                output::print_json(&resp);
            } else {
                commands::misc::format_commands(&resp);
            }
        }
        Commands::Tags => {
            let resp = moat.get("/api/v1/tags").await?;
            if cli.json {
                output::print_json(&resp);
            } else {
                commands::misc::format_tags(&resp);
            }
        }
    }

    Ok(())
}

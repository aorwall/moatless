#![expect(clippy::print_stderr, reason = "the CLI reports errors on stderr")]
//! `moat` — CLI for the Moatless backend API.
//!
//! Two modes selected at startup by the `USER_AUTH_TOKEN` env var:
//! - **Sandbox** (inside a pod): no login/logout, adds `set-name` / `set-tags`
//! - **Standalone** (human user): login/logout, no sandbox-only commands

mod args;
mod compose;
mod invocation;

use args::{SandboxCli, StandaloneCli};
use clap::Parser;

#[tokio::main]
async fn main() {
    let is_sandbox = moatless_cli::config::sandbox_token().is_some();

    let result = if is_sandbox {
        let cli = SandboxCli::parse();
        invocation::run_sandbox(cli).await
    } else {
        let cli = StandaloneCli::parse();
        invocation::run_standalone(cli).await
    };

    if let Err(e) = result {
        if let Some(status) = e.downcast_ref::<moatless_cli::commands::CommandExit>() {
            std::process::exit(status.0);
        }
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

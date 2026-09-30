//! Dispatch parsed CLI commands using the selected authentication mode.

use crate::args::{SandboxCli, SandboxCommands, SharedCommands, StandaloneCli, StandaloneCommands};
use moatless_cli::{auth, client::MoatClient, commands, config::Config};

pub(super) async fn run_sandbox(cli: SandboxCli) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env();
    let mut moat = crate::compose::client(&cfg, cli.opts.verbose, cli.opts.timeout);

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
            commands::tasks::set_name(moat, cfg, name, json).await?;
        }
        SandboxCommands::SetTags { tags } => {
            commands::tasks::set_tags(moat, cfg, tags, json).await?;
        }
        SandboxCommands::Cmd { action } => {
            let task_id = cfg.require_task_id()?;
            commands::cmd::run(action, task_id, json, &cfg.sandbox_grpc_port).await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Standalone dispatch
// ---------------------------------------------------------------------------

pub(super) async fn run_standalone(cli: StandaloneCli) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env();
    let mut moat = crate::compose::client(&cfg, cli.opts.verbose, cli.opts.timeout);

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
        StandaloneCommands::Logout => auth::logout(&cfg),
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
        SharedCommands::Whoami => commands::info::whoami(moat, json).await?,
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
            commands::repos::run(moat, *action, json, cfg.github_token_set).await?;
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
        SharedCommands::Tags => commands::info::tags(moat, json).await?,
        SharedCommands::K8s { action } => {
            commands::k8s::run(action, json).await?;
        }
        SharedCommands::Docker { action } => {
            commands::docker::run(action, json).await?;
        }
        SharedCommands::User { action } => {
            commands::users::run(moat, action, json).await?;
        }
        SharedCommands::Bot { action } => {
            commands::bots::run(moat, action, json).await?;
        }
        SharedCommands::GithubApp { action } => {
            commands::github_apps::run(moat, action, json).await?;
        }
        SharedCommands::App { action } => {
            commands::provider_apps::run(moat, action, json).await?;
        }
        SharedCommands::Send(args) => {
            commands::send::run(moat, *args, cfg, json).await?;
        }
        SharedCommands::Messages { action } => {
            commands::messages::run(moat, action, json).await?;
        }
        SharedCommands::Feedback(args) => {
            commands::feedback::dispatch(moat, cfg, args, json).await?;
        }
        SharedCommands::Decide { action } => {
            commands::decide::run(moat, action, cfg, json).await?;
        }
        SharedCommands::Artifact { action } => {
            commands::artifact::run(moat, action, cfg, json).await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `moat cmd` talks only to the sandbox's own loopback gRPC server, so it must
/// keep working when the backend session is stale — that is exactly when a
/// build still needs supervising.
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

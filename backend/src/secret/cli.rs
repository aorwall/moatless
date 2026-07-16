//! `moatless-vibe-backend secrets …` CLI entry.
//!
//! The CLI reuses the same binary and the same `SecretService`/`LocalEnvelopeSecretCipher`
//! the HTTP server uses. It runs as an admin `ScopeContext` (actor id `cli`).

// reason: CLI subcommand writes results to stdout by design.
#![allow(clippy::print_stdout)]

use std::io::{self, Read};
use std::sync::Arc;

use clap::{Args, Subcommand, ValueEnum};

use super::audit::PgSecretAuditRepository;
use super::cli_migrate::{run_migrate_repo_env_vars, MigrateOptions};
use super::dao::PgSecretRepository;
use super::models::SecretScopeType;
use super::schemas::PutSecretRequest;
use super::service::{LocalEnvelopeSecretCipher, SecretService};
use crate::config::Config;
use crate::db;
use moatless_backend_core::scope::ScopeContext;

#[derive(Args, Debug)]
pub struct SecretsCli {
    #[command(subcommand)]
    pub command: SecretsCommand,
}

#[derive(Subcommand, Debug)]
pub enum SecretsCommand {
    /// List secret metadata for a given scope.
    List(ListArgs),
    /// Create or replace a secret value.
    Put(PutArgs),
    /// Delete a secret by scope + key.
    Delete(DeleteArgs),
    /// Migrate secret-like repository env_vars into the scoped secrets table.
    MigrateRepoEnvVars(MigrateArgs),
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum CliScope {
    Global,
    Team,
    User,
}

impl From<CliScope> for SecretScopeType {
    fn from(value: CliScope) -> Self {
        match value {
            CliScope::Global => Self::Global,
            CliScope::Team => Self::Team,
            CliScope::User => Self::User,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum CliKind {
    Env,
    ProviderToken,
    RuntimeFile,
    Deployment,
    SshKey,
}

impl From<CliKind> for super::models::SecretKind {
    fn from(value: CliKind) -> Self {
        match value {
            CliKind::Env => Self::Env,
            CliKind::ProviderToken => Self::ProviderToken,
            CliKind::RuntimeFile => Self::RuntimeFile,
            CliKind::Deployment => Self::Deployment,
            CliKind::SshKey => Self::SshKey,
        }
    }
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long, value_enum)]
    pub scope: CliScope,
    #[arg(long)]
    pub scope_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct PutArgs {
    #[arg(long, value_enum)]
    pub scope: CliScope,
    #[arg(long)]
    pub scope_id: Option<String>,
    #[arg(long)]
    pub key: String,
    #[arg(long, value_enum, default_value_t = CliKind::Env)]
    pub kind: CliKind,
    #[arg(long, conflicts_with = "value_stdin")]
    pub value: Option<String>,
    /// Read the secret value from stdin (single trimmed line).
    #[arg(long)]
    pub value_stdin: bool,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    #[arg(long, value_enum)]
    pub scope: CliScope,
    #[arg(long)]
    pub scope_id: Option<String>,
    #[arg(long)]
    pub key: String,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Print the plan without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Keys whose identical value appears in at least N repos are promoted to global.
    #[arg(long, default_value_t = 3)]
    pub global_threshold: usize,
}

const CLI_ACTOR: &str = "cli";

/// Entry point. Parses no args — the caller passes a pre-parsed [`SecretsCli`].
pub async fn run(cli: SecretsCli) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let url = config
        .database_url
        .as_deref()
        .ok_or("DATABASE_URL must be set for secrets CLI")?;
    let pool = db::create_pool(url, config.database.max_connections).await?;
    let secrets_repo = Arc::new(PgSecretRepository(pool.clone()));
    let audit_repo = Arc::new(PgSecretAuditRepository(pool.clone()));
    let cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets)
        .ok_or("SECRETS_ENCRYPTION_KEY must be set (base64 or raw 32 bytes) for secrets CLI")?;
    let service = SecretService::new(secrets_repo, audit_repo, Some(cipher));
    let ctx = admin_context();

    match cli.command {
        SecretsCommand::List(args) => list(&service, &ctx, args).await?,
        SecretsCommand::Put(args) => put(&service, &ctx, args).await?,
        SecretsCommand::Delete(args) => delete(&service, &ctx, args).await?,
        SecretsCommand::MigrateRepoEnvVars(args) => {
            if !args.dry_run && std::env::var("MOATLESS_CLI_CONFIRM").ok().as_deref() != Some("yes")
            {
                return Err(
                    "refusing to run without MOATLESS_CLI_CONFIRM=yes (or pass --dry-run)".into(),
                );
            }
            run_migrate_repo_env_vars(
                &pool,
                &service,
                &ctx,
                CLI_ACTOR,
                MigrateOptions {
                    dry_run: args.dry_run,
                    global_threshold: args.global_threshold,
                },
            )
            .await?;
        }
    }
    Ok(())
}

fn admin_context() -> ScopeContext {
    ScopeContext {
        user_id: CLI_ACTOR.to_string(),
        team_ids: Vec::new(),
        is_admin: true,
    }
}

async fn list(
    service: &SecretService,
    ctx: &ScopeContext,
    args: ListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = service
        .list_metadata(ctx, args.scope.into(), args.scope_id.as_deref())
        .await?;
    if rows.is_empty() {
        println!("(no secrets)");
        return Ok(());
    }
    println!(
        "{:<44}  {:<10}  {:<32}  {:<24}  updated",
        "ID", "SCOPE", "KEY", "SCOPE_ID"
    );
    for row in rows {
        println!(
            "{:<44}  {:<10}  {:<32}  {:<24}  {}",
            row.id,
            row.scope_type,
            row.key,
            row.scope_id.clone().unwrap_or_default(),
            row.updated_at,
        );
    }
    Ok(())
}

async fn put(
    service: &SecretService,
    ctx: &ScopeContext,
    args: PutArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let value = resolve_value(args.value, args.value_stdin)?;
    let req = PutSecretRequest {
        scope_type: args.scope.into(),
        scope_id: args.scope_id,
        key: args.key,
        kind: args.kind.into(),
        provider: None,
        value: Some(value),
        external_ref: None,
    };
    let row = service.put_secret(ctx, CLI_ACTOR, req).await?;
    println!("ok: secret {} @ {}", row.key, row.scope_type);
    Ok(())
}

async fn delete(
    service: &SecretService,
    ctx: &ScopeContext,
    args: DeleteArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = service
        .list_metadata(ctx, args.scope.into(), args.scope_id.as_deref())
        .await?;
    let match_id = rows
        .iter()
        .find(|row| row.key == args.key)
        .map(|row| row.id.clone());
    match match_id {
        Some(id) => {
            service.delete_secret(ctx, CLI_ACTOR, &id).await?;
            println!("ok: deleted {id}");
        }
        None => {
            println!("(no match: {})", args.key);
        }
    }
    Ok(())
}

fn resolve_value(
    value: Option<String>,
    from_stdin: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, from_stdin) {
        (Some(v), false) => Ok(v),
        (None, true) => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf.trim_end_matches('\n').to_string())
        }
        (None, false) => Err("one of --value or --value-stdin is required".into()),
        (Some(_), true) => Err("--value and --value-stdin are mutually exclusive".into()),
    }
}

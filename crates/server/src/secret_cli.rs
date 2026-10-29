//! `moatless-backend secrets …` CLI entry.
//!
//! The CLI reuses the same binary and the same
//! `SecretService`/`LocalEnvelopeSecretCipher` the HTTP server uses. Its
//! `ScopeContext` comes from [`scope_context`]: admin when no owner is named,
//! and the named owner otherwise. The audit actor is `cli` either way.

// reason: CLI subcommand writes results to stdout by design.
#![allow(clippy::print_stdout)]

use std::io::{self, Read};

use clap::{Args, Subcommand, ValueEnum};

use moatless_api_schemas::enums::Scope;
use moatless_core::config::Config;
use moatless_identity::secret::schemas::PutSecretRequest;
use moatless_identity::secret::service::SecretService;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::repository::secret_migration::{
    MigrateOptions, run_migrate_repo_env_vars,
};

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
    /// Migrate secret-like repository `env_vars` into the scoped secrets table.
    MigrateRepoEnvVars(MigrateArgs),
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum CliScope {
    Global,
    User,
}

impl From<CliScope> for Scope {
    fn from(value: CliScope) -> Self {
        match value {
            CliScope::Global => Self::Global,
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
}

impl From<CliKind> for moatless_types::secret::SecretKind {
    fn from(value: CliKind) -> Self {
        match value {
            CliKind::Env => Self::Env,
            CliKind::ProviderToken => Self::ProviderToken,
            CliKind::RuntimeFile => Self::RuntimeFile,
            CliKind::Deployment => Self::Deployment,
        }
    }
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long, value_enum)]
    pub scope: CliScope,
    /// Owner user ID (required for user scope).
    #[arg(long)]
    pub owner_user_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct PutArgs {
    #[arg(long, value_enum)]
    pub scope: CliScope,
    /// Owner user ID (required for user scope).
    #[arg(long)]
    pub owner_user_id: Option<String>,
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
    /// Owner user ID (required for user scope).
    #[arg(long)]
    pub owner_user_id: Option<String>,
    #[arg(long)]
    pub key: String,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Print the plan without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Keys whose identical value appears in at least N repos are promoted to
    /// global.
    #[arg(long, default_value_t = 3)]
    pub global_threshold: usize,
    /// Must be `yes` for a run that writes.
    #[arg(long, env = "MOATLESS_CLI_CONFIRM")]
    pub confirm: Option<String>,
}

const CLI_ACTOR: &str = "cli";

/// Entry point. Parses no args — the caller passes a pre-parsed [`SecretsCli`].
pub async fn run(cli: SecretsCli) -> Result<(), Box<dyn std::error::Error>> {
    crate::startup::init_tracing(&moatless_core::config::OtelConfig::from_env());
    let config = Config::from_env()?;
    let (pool, service) = crate::compose::secrets::build(&config).await?;

    match cli.command {
        SecretsCommand::List(args) => list(&service, args).await?,
        SecretsCommand::Put(args) => put(&service, args).await?,
        SecretsCommand::Delete(args) => delete(&service, args).await?,
        SecretsCommand::MigrateRepoEnvVars(args) => {
            if !args.dry_run && args.confirm.as_deref() != Some("yes") {
                return Err(
                    "refusing to run without MOATLESS_CLI_CONFIRM=yes (or pass --dry-run)".into(),
                );
            }
            run_migrate_repo_env_vars(
                &pool,
                &service,
                &scope_context(None),
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

/// The context a subcommand acts under.
///
/// Naming an owner runs *as* them (`is_admin: false`), scoping reads/deletes to
/// that user's rows plus the global ones, so `--owner-user-id` can't reach a
/// third user's secret. The audit actor stays [`CLI_ACTOR`], passed separately.
fn scope_context(owner_user_id: Option<&str>) -> ScopeContext {
    ScopeContext::new(
        owner_user_id.unwrap_or(CLI_ACTOR).to_string(),
        owner_user_id.is_none(),
    )
}

/// A user-scoped row is owned by exactly one user and a global row by nobody,
/// so the flag is required for one scope and rejected for the other.
///
/// Without this, `--scope user` silently wrote `owner_user_id = 'cli'`, which
/// no user's secret resolution ever matches.
fn owner_for(
    scope: Scope,
    owner_user_id: Option<String>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match (scope, owner_user_id) {
        (Scope::User, None) => Err("--owner-user-id is required with --scope user".into()),
        // `--owner-user-id "$USER_ID"` on an unset variable reaches clap as an
        // empty string, not as absent, and would write the same unreachable row
        // this function exists to prevent.
        (Scope::User, Some(owner)) if owner.trim().is_empty() => {
            Err("--owner-user-id cannot be empty".into())
        }
        (Scope::User, owner) => Ok(owner),
        (Scope::Global, Some(_)) => Err("--owner-user-id is not valid with --scope global".into()),
        (Scope::Global, None) => Ok(None),
    }
}

async fn list(service: &SecretService, args: ListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scope: Scope = args.scope.into();
    let owner = owner_for(scope, args.owner_user_id)?;
    let rows = service
        .list_metadata(&scope_context(owner.as_deref()), Some(scope))
        .await?;
    if rows.is_empty() {
        println!("(no secrets)");
        return Ok(());
    }
    println!(
        "{:<44}  {:<10}  {:<32}  {:<24}  updated",
        "ID", "SCOPE", "KEY", "OWNER_USER_ID"
    );
    for row in rows {
        println!(
            "{:<44}  {:<10}  {:<32}  {:<24}  {}",
            row.id,
            row.scope,
            row.key,
            row.owner_user_id.clone().unwrap_or_default(),
            row.updated_at,
        );
    }
    Ok(())
}

async fn put(service: &SecretService, args: PutArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scope: Scope = args.scope.into();
    let owner = owner_for(scope, args.owner_user_id)?;
    let value = resolve_value(args.value, args.value_stdin)?;
    let req = PutSecretRequest {
        scope: Some(scope),
        key: args.key,
        kind: args.kind.into(),
        provider: None,
        value: Some(value),
        external_ref: None,
    };
    let row = service
        .put_secret(&scope_context(owner.as_deref()), CLI_ACTOR, req)
        .await?;
    println!(
        "ok: secret {} @ {} {}",
        row.key,
        row.scope,
        row.owner_user_id.as_deref().unwrap_or("")
    );
    Ok(())
}

async fn delete(
    service: &SecretService,
    args: DeleteArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let scope: Scope = args.scope.into();
    let owner = owner_for(scope, args.owner_user_id)?;
    let ctx = &scope_context(owner.as_deref());
    let rows = service.list_metadata(ctx, Some(scope)).await?;
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

#[cfg(test)]
mod tests {
    use super::{owner_for, scope_context};
    use moatless_api_schemas::enums::Scope;
    use moatless_types::access_control::resolve_scope_for_create;

    const ALBIN: &str = "8b925a56-eb87-46a0-b81e-8ebeb3393a66";

    #[test]
    fn a_named_owner_owns_the_row_put_creates() {
        let ctx = scope_context(Some(ALBIN));
        let resolved = resolve_scope_for_create(&ctx, Some(Scope::User)).expect("resolve");
        assert_eq!(resolved.scope, Scope::User);
        assert_eq!(resolved.owner_user_id.as_deref(), Some(ALBIN));
    }

    #[test]
    fn a_named_owner_drops_admin_so_reads_stay_scoped_to_them() {
        let ctx = scope_context(Some(ALBIN));
        assert!(!ctx.is_admin);
        assert!(scope_context(None).is_admin);
    }

    #[test]
    fn global_puts_still_run_as_the_admin_cli_actor() {
        let ctx = scope_context(None);
        let resolved = resolve_scope_for_create(&ctx, Some(Scope::Global)).expect("resolve");
        assert_eq!(resolved.scope, Scope::Global);
        assert_eq!(resolved.owner_user_id, None);
    }

    #[test]
    fn owner_is_required_for_user_scope_and_rejected_for_global() {
        assert!(owner_for(Scope::User, None).is_err());
        assert_eq!(
            owner_for(Scope::User, Some(ALBIN.to_string()))
                .expect("user scope")
                .as_deref(),
            Some(ALBIN)
        );
        assert!(owner_for(Scope::Global, Some(ALBIN.to_string())).is_err());
        assert_eq!(owner_for(Scope::Global, None).expect("global scope"), None);
    }

    #[test]
    fn a_blank_owner_is_rejected_rather_than_owning_a_row() {
        assert!(owner_for(Scope::User, Some(String::new())).is_err());
        assert!(owner_for(Scope::User, Some("  ".to_string())).is_err());
    }
}

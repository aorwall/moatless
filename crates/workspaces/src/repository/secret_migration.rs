#![allow(
    clippy::print_stdout,
    reason = "CLI subcommand writes a plan summary to stdout"
)]
//! `secrets migrate-repo-env-vars` implementation.
//!
//! Scans `repositories.env_vars`, moves secret-looking keys into the encrypted
//! `secrets` table, and strips the migrated keys from the repo rows.

use super::secret_migration_dao::{self, RepoEnvRow};
use moatless_core::db::DbPool;
use moatless_core::secret_keys::looks_like_secret_key;
use moatless_identity::secret::schemas::PutSecretRequest;
use moatless_identity::secret::service::SecretService;
use moatless_types::access_control::Scope;
use moatless_types::access_control::ScopeContext;
use moatless_types::secret::SecretKind;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub global_threshold: usize,
}

struct KeyPlan {
    key: String,
    groups: Vec<ValueGroup>,
}

struct ValueGroup {
    value: String,
    repos: Vec<RepoRef>,
    action: GroupAction,
}

#[derive(Clone)]
struct RepoRef {
    id: String,
    owner: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum GroupAction {
    Global,
    User,
    SkipNoOwner,
}

pub async fn run_migrate_repo_env_vars(
    pool: &DbPool,
    service: &SecretService,
    ctx: &ScopeContext,
    actor_id: &str,
    opts: MigrateOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = secret_migration_dao::list_repository_env(pool).await?;

    let plans = build_plan(&rows, opts.global_threshold);
    print_plan(&plans, opts);

    if opts.dry_run {
        return Ok(());
    }

    for plan in &plans {
        for group in &plan.groups {
            match group.action {
                GroupAction::Global => {
                    put_global(service, ctx, actor_id, &plan.key, &group.value).await?;
                    strip_from_repos(pool, &plan.key, &group.repos).await?;
                }
                GroupAction::User => {
                    for repo in &group.repos {
                        let Some(owner) = repo.owner.as_deref() else {
                            continue;
                        };
                        put_user(service, actor_id, owner, &plan.key, &group.value).await?;
                        strip_from_repos(pool, &plan.key, std::slice::from_ref(repo)).await?;
                    }
                }
                GroupAction::SkipNoOwner => {}
            }
        }
    }

    println!("migration complete");
    Ok(())
}

fn build_plan(rows: &[RepoEnvRow], global_threshold: usize) -> Vec<KeyPlan> {
    // key -> value -> repos
    let mut by_key: BTreeMap<String, BTreeMap<String, Vec<RepoRef>>> = BTreeMap::new();

    for row in rows {
        let Some(map) = row.env_vars.as_ref().and_then(|v| v.as_object()) else {
            continue;
        };
        for (key, value) in map {
            if !looks_like_secret_key(key) {
                continue;
            }
            let Some(value_str) = value.as_str() else {
                continue;
            };
            by_key
                .entry(key.clone())
                .or_default()
                .entry(value_str.to_string())
                .or_default()
                .push(RepoRef {
                    id: row.id.clone(),
                    owner: row.created_by.clone(),
                });
        }
    }

    let mut plans = Vec::new();
    for (key, values) in by_key {
        let mut groups = Vec::new();
        for (value, repos) in values {
            let action = if repos.len() >= global_threshold {
                GroupAction::Global
            } else if repos.iter().any(|r| r.owner.is_some()) {
                GroupAction::User
            } else {
                GroupAction::SkipNoOwner
            };
            groups.push(ValueGroup {
                value,
                repos,
                action,
            });
        }
        plans.push(KeyPlan { key, groups });
    }
    plans
}

fn print_plan(plans: &[KeyPlan], opts: MigrateOptions) {
    if plans.is_empty() {
        println!("no secret-like keys found in any repositories.env_vars");
        return;
    }
    println!(
        "plan (dry_run={}, global_threshold={}):",
        opts.dry_run, opts.global_threshold
    );
    for plan in plans {
        for (idx, group) in plan.groups.iter().enumerate() {
            let tag = if idx == 0 { &plan.key } else { "" };
            let action = match group.action {
                GroupAction::Global => "→ global",
                GroupAction::User => "→ user",
                GroupAction::SkipNoOwner => "skip (no owner)",
            };
            println!("  {:<32}  {:>4} repos   {}", tag, group.repos.len(), action);
        }
    }
}

async fn put_global(
    service: &SecretService,
    ctx: &ScopeContext,
    actor_id: &str,
    key: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    service
        .put_secret(
            ctx,
            actor_id,
            PutSecretRequest {
                scope: Some(Scope::Global),
                key: key.to_string(),
                kind: SecretKind::Env,
                provider: None,
                value: Some(value.to_string()),
                external_ref: None,
            },
        )
        .await?;
    Ok(())
}

async fn put_user(
    service: &SecretService,
    actor_id: &str,
    owner_id: &str,
    key: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // For user-scoped migrate we create via the admin context which allows any scope.
    // The owner_id is set as the ctx user for resolution via resolve_scope_for_create.
    // We use a temporary ctx to impersonate the owner.
    let owner_ctx = moatless_types::access_control::ScopeContext::user(owner_id.to_string());
    service
        .put_secret(
            &owner_ctx,
            actor_id,
            PutSecretRequest {
                scope: Some(Scope::User),
                key: key.to_string(),
                kind: SecretKind::Env,
                provider: None,
                value: Some(value.to_string()),
                external_ref: None,
            },
        )
        .await?;
    Ok(())
}

async fn strip_from_repos(pool: &DbPool, key: &str, repos: &[RepoRef]) -> Result<(), sqlx::Error> {
    if repos.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = repos.iter().map(|r| r.id.as_str()).collect();
    secret_migration_dao::strip_keys(pool, key, &ids).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str, owner: Option<&str>, kv: serde_json::Value) -> RepoEnvRow {
        RepoEnvRow {
            id: id.to_string(),
            created_by: owner.map(str::to_string),
            env_vars: Some(kv),
        }
    }

    #[test]
    fn promotes_shared_values_to_global() {
        let rows = vec![
            row(
                "r1",
                Some("u1"),
                json!({"OPENAI_API_KEY": "sk-abc", "NODE_ENV": "prod"}),
            ),
            row("r2", Some("u2"), json!({"OPENAI_API_KEY": "sk-abc"})),
            row("r3", Some("u3"), json!({"OPENAI_API_KEY": "sk-abc"})),
        ];
        let plans = build_plan(&rows, 3);
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(plan.key, "OPENAI_API_KEY");
        assert_eq!(plan.groups.len(), 1);
        assert!(matches!(plan.groups[0].action, GroupAction::Global));
    }

    #[test]
    fn isolates_unique_values_to_user_scope() {
        let rows = vec![
            row("r1", Some("u1"), json!({"STRIPE_SECRET_KEY": "sk_live_a"})),
            row("r2", Some("u2"), json!({"STRIPE_SECRET_KEY": "sk_live_b"})),
        ];
        let plans = build_plan(&rows, 3);
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(plan.groups.len(), 2);
        assert!(
            plan.groups
                .iter()
                .all(|g| matches!(g.action, GroupAction::User))
        );
    }

    #[test]
    fn skips_non_secret_keys() {
        let rows = vec![row(
            "r1",
            Some("u1"),
            json!({"NODE_ENV": "prod", "PUBLIC_URL": "https://x"}),
        )];
        let plans = build_plan(&rows, 3);
        assert!(plans.is_empty());
    }

    #[test]
    fn skips_owner_less_repos_when_below_threshold() {
        let rows = vec![row("r1", None, json!({"OPENAI_API_KEY": "sk-abc"}))];
        let plans = build_plan(&rows, 3);
        assert_eq!(plans.len(), 1);
        assert!(matches!(
            plans[0].groups[0].action,
            GroupAction::SkipNoOwner
        ));
    }
}

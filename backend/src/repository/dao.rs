//! Repository DAO — queries for repository data access.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::db::DbPool;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};

/// Patch struct for partial updates.
#[derive(Default)]
pub struct RepositoryPatch {
    pub name: Option<String>,
    pub kind: Option<RepositoryKind>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
    pub owner_user_id: Option<Option<String>>,
    pub remote_url: Option<Option<String>>,
    pub default_branch: Option<Option<String>>,
    pub docker_image: Option<Option<String>>,
    pub docker_access: Option<Option<bool>>,
    pub sandbox_service_account: Option<Option<String>>,
    pub network_profile: Option<Option<String>>,
    pub system_prompt: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub provider: Option<Option<RepositoryProvider>>,
    pub servers: Option<Option<serde_json::Value>>,
    pub plugins: Option<Option<serde_json::Value>>,
    pub env_vars: Option<Option<serde_json::Value>>,
    pub setup_commands: Option<Option<serde_json::Value>>,
    pub server_start_commands: Option<Option<serde_json::Value>>,
    pub startup_timeout_seconds: Option<Option<i32>>,
    pub build_command: Option<Option<String>>,
    pub build_output_directory: Option<Option<String>>,
    pub enable_inspector: Option<Option<bool>>,
    pub auto_reload: Option<Option<bool>>,
    pub resources: Option<Option<serde_json::Value>>,
    pub access_status: Option<String>,
    pub access_checked_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub access_error: Option<Option<String>>,
}

/// Repository trait for repository persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RepoRepository: Send + Sync {
    /// Find by id (internal, no access control).
    async fn find_repository_by_id(&self, id: &str) -> Result<Option<Repository>, sqlx::Error>;
    /// Find by id, scoped to access control.
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Repository>, sqlx::Error>;
    async fn list_all(&self, ctx: &ScopeContext) -> Result<Vec<Repository>, sqlx::Error>;
    async fn insert(&self, row: &Repository) -> Result<(), sqlx::Error>;
    async fn update(&self, id: &str, patch: &RepositoryPatch) -> Result<bool, sqlx::Error>;
    async fn soft_delete(&self, id: &str) -> Result<bool, sqlx::Error>;
    async fn find_by_name_and_user(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Option<Repository>, sqlx::Error>;
    /// Find a non-deleted repository whose `remote_url` normalizes to `url_key`
    /// (trailing `/`/`.git` stripped, lowercased), scoped to the given
    /// scope/team/owner so resolution never crosses tenants. `url_key` must be
    /// pre-normalized by [`super::handlers::normalize_remote_url`].
    async fn find_by_normalized_remote_url(
        &self,
        url_key: &str,
        scope: Scope,
        team_id: Option<String>,
        owner_user_id: Option<String>,
    ) -> Result<Option<Repository>, sqlx::Error>;
    async fn find_names_by_ids(&self, ids: &[String])
        -> Result<Vec<(String, String)>, sqlx::Error>;
    /// Check if a repository exists by name and creator, **including** soft-deleted rows.
    async fn exists_by_name_including_deleted(
        &self,
        name: &str,
        created_by: &str,
    ) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`RepoRepository`].
pub struct PgRepoRepository(pub DbPool);

#[async_trait]
impl RepoRepository for PgRepoRepository {
    async fn find_repository_by_id(&self, id: &str) -> Result<Option<Repository>, sqlx::Error> {
        sqlx::query_as::<_, Repository>(
            "SELECT * FROM repositories WHERE id = $1 AND (deleted IS NULL OR deleted = false)",
        )
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Repository>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Repository, "", 2);
        let sql = format!(
            "SELECT * FROM repositories \
             WHERE id = $1 \
             AND (deleted IS NULL OR deleted = false) \
             AND {}",
            access.sql()
        );
        sqlx::query_as::<_, Repository>(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_all(&self, ctx: &ScopeContext) -> Result<Vec<Repository>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Repository, "", 1);
        let sql = format!(
            "SELECT * FROM repositories \
             WHERE (deleted IS NULL OR deleted = false) \
             AND {} \
             ORDER BY name ASC",
            access.sql()
        );
        sqlx::query_as::<_, Repository>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    async fn insert(&self, row: &Repository) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO repositories (
                id, name, kind, remote_url, default_branch,
                docker_image, system_prompt, setup_commands, env_vars,
                servers, plugins,
                server_start_commands, startup_timeout_seconds,
                build_command, build_output_directory,
                enable_inspector, auto_reload,
                description, provider, scope, team_id, owner_user_id, created_by,
                resources,
                deleted, created_at, updated_at,
                docker_access, sandbox_service_account, network_profile,
                access_status, access_checked_at, access_error
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9,
                $10, $11,
                $12, $13,
                $14, $15,
                $16, $17,
                $18, $19, $20, $21, $22, $23,
                $24,
                $25, $26, $27,
                $28, $29, $30,
                $31, $32, $33
            )"#,
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(row.kind)
        .bind(&row.remote_url)
        .bind(&row.default_branch)
        .bind(&row.docker_image)
        .bind(&row.system_prompt)
        .bind(&row.setup_commands)
        .bind(&row.env_vars)
        .bind(&row.servers)
        .bind(&row.plugins)
        .bind(&row.server_start_commands)
        .bind(row.startup_timeout_seconds)
        .bind(&row.build_command)
        .bind(&row.build_output_directory)
        .bind(row.enable_inspector)
        .bind(row.auto_reload)
        .bind(&row.description)
        .bind(row.provider)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(&row.resources)
        .bind(row.deleted)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(row.docker_access)
        .bind(&row.sandbox_service_account)
        .bind(&row.network_profile)
        .bind(&row.access_status)
        .bind(row.access_checked_at)
        .bind(&row.access_error)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn update(&self, id: &str, patch: &RepositoryPatch) -> Result<bool, sqlx::Error> {
        let mut sets: Vec<String> = Vec::new();
        let mut idx = 2; // $1 is id

        macro_rules! push_set {
            ($field:ident) => {
                if patch.$field.is_some() {
                    sets.push(format!("{} = ${idx}", stringify!($field)));
                    idx += 1;
                }
            };
        }

        push_set!(name);
        push_set!(kind);
        push_set!(scope);
        push_set!(team_id);
        push_set!(owner_user_id);
        push_set!(remote_url);
        push_set!(default_branch);
        push_set!(docker_image);
        push_set!(docker_access);
        push_set!(sandbox_service_account);
        push_set!(network_profile);
        push_set!(system_prompt);
        push_set!(description);
        push_set!(provider);
        push_set!(servers);
        push_set!(plugins);
        push_set!(env_vars);
        push_set!(setup_commands);
        push_set!(server_start_commands);
        push_set!(startup_timeout_seconds);
        push_set!(build_command);
        push_set!(build_output_directory);
        push_set!(enable_inspector);
        push_set!(auto_reload);
        push_set!(resources);
        push_set!(access_status);
        push_set!(access_checked_at);
        push_set!(access_error);

        if sets.is_empty() {
            return Ok(false);
        }

        sets.push(format!("updated_at = ${idx}"));

        let sql = format!(
            "UPDATE repositories SET {} WHERE id = $1 AND (deleted IS NULL OR deleted = false)",
            sets.join(", ")
        );

        let now = chrono::Utc::now();
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind(id);

        macro_rules! bind_opt {
            ($field:ident) => {
                if let Some(ref val) = patch.$field {
                    q = q.bind(val);
                }
            };
        }

        bind_opt!(name);
        bind_opt!(kind);
        bind_opt!(scope);
        bind_opt!(team_id);
        bind_opt!(owner_user_id);
        bind_opt!(remote_url);
        bind_opt!(default_branch);
        bind_opt!(docker_image);
        bind_opt!(docker_access);
        bind_opt!(sandbox_service_account);
        bind_opt!(network_profile);
        bind_opt!(system_prompt);
        bind_opt!(description);
        bind_opt!(provider);
        bind_opt!(servers);
        bind_opt!(plugins);
        bind_opt!(env_vars);
        bind_opt!(setup_commands);
        bind_opt!(server_start_commands);
        bind_opt!(startup_timeout_seconds);
        bind_opt!(build_command);
        bind_opt!(build_output_directory);
        bind_opt!(enable_inspector);
        bind_opt!(auto_reload);
        bind_opt!(resources);
        bind_opt!(access_status);
        bind_opt!(access_checked_at);
        bind_opt!(access_error);

        // Bind updated_at
        q = q.bind(now);

        let result = q.execute(&self.0).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn soft_delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE repositories SET deleted = true, updated_at = $2 WHERE id = $1 AND (deleted IS NULL OR deleted = false)",
        )
        .bind(id)
        .bind(chrono::Utc::now())
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn find_by_name_and_user(
        &self,
        name: &str,
        user_id: &str,
    ) -> Result<Option<Repository>, sqlx::Error> {
        sqlx::query_as::<_, Repository>(
            "SELECT * FROM repositories \
             WHERE name = $1 AND created_by = $2 \
             AND (deleted IS NULL OR deleted = false)",
        )
        .bind(name)
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_normalized_remote_url(
        &self,
        url_key: &str,
        scope: Scope,
        team_id: Option<String>,
        owner_user_id: Option<String>,
    ) -> Result<Option<Repository>, sqlx::Error> {
        // Normalize the stored URL the same way `normalize_remote_url` does:
        // strip trailing slashes, drop a trailing `.git`, strip trailing slashes
        // again, then lowercase — so the comparison matches the pre-normalized
        // `url_key`. Scoped to (scope, team, owner) to keep resolution within a
        // single tenant.
        sqlx::query_as::<_, Repository>(
            r#"SELECT * FROM repositories
               WHERE (deleted IS NULL OR deleted = false)
                 AND lower(
                       regexp_replace(
                         regexp_replace(
                           regexp_replace(coalesce(remote_url, ''), '/+$', ''),
                           '\.git$', ''),
                         '/+$', '')
                     ) = $1
                 AND scope = $2
                 AND team_id IS NOT DISTINCT FROM $3
                 AND owner_user_id IS NOT DISTINCT FROM $4
               ORDER BY created_at ASC NULLS LAST
               LIMIT 1"#,
        )
        .bind(url_key)
        .bind(scope)
        .bind(team_id)
        .bind(owner_user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_names_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, (String, String)>(
            "SELECT id, name FROM repositories WHERE id = ANY($1) AND (deleted IS NULL OR deleted = false)"
        )
        .bind(ids)
        .fetch_all(&self.0)
        .await
    }

    async fn exists_by_name_including_deleted(
        &self,
        name: &str,
        created_by: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM repositories WHERE name = $1 AND created_by = $2 LIMIT 1",
        )
        .bind(name)
        .bind(created_by)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.is_some())
    }
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================

/// Insert a repository (convenience wrapper).
pub async fn insert(pool: &DbPool, row: &Repository) -> Result<(), sqlx::Error> {
    PgRepoRepository(pool.clone()).insert(row).await
}

/// Find by id scoped to access control (convenience wrapper).
pub async fn find_by_id(
    pool: &DbPool,
    id: &str,
    ctx: &ScopeContext,
) -> Result<Option<Repository>, sqlx::Error> {
    PgRepoRepository(pool.clone()).find_by_id(id, ctx).await
}

/// List all repositories visible to user (convenience wrapper).
pub async fn list_all(pool: &DbPool, ctx: &ScopeContext) -> Result<Vec<Repository>, sqlx::Error> {
    PgRepoRepository(pool.clone()).list_all(ctx).await
}

/// Update repository fields (convenience wrapper).
pub async fn update(pool: &DbPool, id: &str, patch: &RepositoryPatch) -> Result<bool, sqlx::Error> {
    PgRepoRepository(pool.clone()).update(id, patch).await
}

/// Soft-delete a repository (convenience wrapper).
pub async fn soft_delete(pool: &DbPool, id: &str) -> Result<bool, sqlx::Error> {
    PgRepoRepository(pool.clone()).soft_delete(id).await
}

/// Find by name and user (convenience wrapper).
pub async fn find_by_name_and_user(
    pool: &DbPool,
    name: &str,
    user_id: &str,
) -> Result<Option<Repository>, sqlx::Error> {
    PgRepoRepository(pool.clone())
        .find_by_name_and_user(name, user_id)
        .await
}

//! Repository DAO — queries for repository data access.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{Repository, RepositoryPatch, RepositoryRow};
use moatless_core::db::DbPool;
use moatless_types::access_control::{Scope, ScopeContext, ScopedEntity};
use moatless_types::repository::RepositoryProvider;

/// DAO trait for repository persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait RepositoryDao: Send + Sync {
    /// Find by id, scoped to access control.
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Repository>, sqlx::Error>;
    async fn list_all(&self, ctx: &ScopeContext) -> Result<Vec<Repository>, sqlx::Error>;
    async fn insert(&self, row: &Repository) -> Result<(), sqlx::Error>;
    async fn update(
        &self,
        id: &str,
        patch: &RepositoryPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;
    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;
    /// Find a non-deleted repository whose `remote_url` normalizes to
    /// `url_key`, scoped to the given scope+owner so resolution never crosses
    /// tenants. `url_key` must be pre-normalized by
    /// [`moatless_types::repository::normalize_remote_url`].
    async fn find_by_normalized_remote_url(
        &self,
        url_key: &str,
        scope: Scope,
        owner_user_id: Option<String>,
    ) -> Result<Option<Repository>, sqlx::Error>;
    async fn find_names_by_ids(&self, ids: &[String])
    -> Result<Vec<(String, String)>, sqlx::Error>;
    /// Check for a canonical URL within an access owner, including soft-deleted
    /// rows.
    async fn exists_by_normalized_remote_url_including_deleted(
        &self,
        url_key: &str,
        scope: Scope,
        owner_user_id: Option<String>,
    ) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`RepositoryDao`].
pub(crate) struct PgRepositoryDao(pub(crate) DbPool);

#[async_trait]
impl RepositoryDao for PgRepositoryDao {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Repository>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Repository, "", 2);
        let sql = format!(
            "SELECT * FROM repositories \
             WHERE id = $1 \
             AND (deleted IS NULL OR deleted = false) \
             AND {}",
            access.sql()
        );
        let row: Option<RepositoryRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await?;
        Ok(row.map(Repository::from))
    }

    async fn list_all(&self, ctx: &ScopeContext) -> Result<Vec<Repository>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Repository, "", 1);
        let sql = format!(
            "SELECT * FROM repositories \
             WHERE (deleted IS NULL OR deleted = false) \
             AND {} \
             ORDER BY name ASC",
            access.sql()
        );
        let rows: Vec<RepositoryRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await?;
        Ok(rows.into_iter().map(Repository::from).collect())
    }

    async fn insert(&self, row: &Repository) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO repositories (
                id, name, kind, remote_url, default_branch,
                description, provider, scope, owner_user_id, created_by,
                deleted, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9, $10,
                $11, $12, $13
            )"#,
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(row.kind.as_str())
        .bind(&row.remote_url)
        .bind(&row.default_branch)
        .bind(&row.description)
        .bind(row.provider.map(RepositoryProvider::as_str))
        .bind(row.scope.as_str())
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.deleted)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn update(
        &self,
        id: &str,
        patch: &RepositoryPatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
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
        push_set!(owner_user_id);
        push_set!(remote_url);
        push_set!(default_branch);
        push_set!(description);
        push_set!(provider);

        if sets.is_empty() {
            return Ok(false);
        }

        sets.push(format!("updated_at = ${idx}"));
        idx += 1;

        let control = moatless_core::db::access_control::write_filter("", idx);
        let sql = format!(
            "UPDATE repositories SET {} \
             WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND {}",
            sets.join(", "),
            control.sql()
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
        if let Some(kind) = patch.kind {
            q = q.bind(kind.as_str());
        }
        if let Some(scope) = patch.scope {
            q = q.bind(scope.as_str());
        }
        bind_opt!(owner_user_id);
        bind_opt!(remote_url);
        bind_opt!(default_branch);
        bind_opt!(description);
        if let Some(provider) = patch.provider {
            q = q.bind(provider.map(RepositoryProvider::as_str));
        }

        // Bind updated_at then write_filter params
        q = q.bind(now);
        q = q.bind(ctx.is_admin);
        q = q.bind(&ctx.user_id);

        let result = q.execute(&self.0).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let control = moatless_core::db::access_control::write_filter("", 3);
        let sql = format!(
            "UPDATE repositories SET deleted = true, updated_at = $2 \
             WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND {}",
            control.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(chrono::Utc::now())
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn find_by_normalized_remote_url(
        &self,
        url_key: &str,
        scope: Scope,
        owner_user_id: Option<String>,
    ) -> Result<Option<Repository>, sqlx::Error> {
        // Normalize the stored URL the same way `normalize_remote_url` does:
        // trim, strip trailing slashes, drop a trailing `.git`, strip trailing
        // slashes again, then lowercase. Scope the lookup to one access owner.
        let row: Option<RepositoryRow> = sqlx::query_as(
            r#"SELECT * FROM repositories
               WHERE (deleted IS NULL OR deleted = false)
                 AND lower(
                       regexp_replace(
                         regexp_replace(
                           regexp_replace(btrim(coalesce(remote_url, '')), '/+$', ''),
                           '\.git$', ''),
                         '/+$', '')
                     ) = $1
                 AND scope = $2
                 AND owner_user_id IS NOT DISTINCT FROM $3
               ORDER BY created_at ASC NULLS LAST
               LIMIT 1"#,
        )
        .bind(url_key)
        .bind(scope.as_str())
        .bind(owner_user_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(Repository::from))
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

    async fn exists_by_normalized_remote_url_including_deleted(
        &self,
        url_key: &str,
        scope: Scope,
        owner_user_id: Option<String>,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"SELECT 1
               FROM repositories
               WHERE lower(
                       regexp_replace(
                         regexp_replace(
                           regexp_replace(btrim(coalesce(remote_url, '')), '/+$', ''),
                           '\.git$', ''),
                         '/+$', '')
                     ) = $1
                 AND scope = $2
                 AND owner_user_id IS NOT DISTINCT FROM $3
               LIMIT 1"#,
        )
        .bind(url_key)
        .bind(scope.as_str())
        .bind(owner_user_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.is_some())
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod db_tests;

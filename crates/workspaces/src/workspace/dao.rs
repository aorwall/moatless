//! Workspace DAO — queries for workspace + workspace_repo persistence.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{Workspace, WorkspaceRepo};
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};
use moatless_core::db::DbPool;

const ACTIVE_OR_GIT_TOMBSTONE: &str = "((deleted IS NULL OR deleted = false) OR \
    (deleted = true AND source = 'manual' AND source_repository_id IS NOT NULL AND source_key IS NOT NULL))";

/// A workspace in a widened listing, and whether the caller may actually read
/// it.
///
/// [`WorkspaceDao::list_readable_including`] returns a union of two sets
/// with different rules behind them: the workspaces the read rule admits, and
/// the workspaces something the caller may see happens to reference. A row from
/// the second half is present so that a listing can render a group for it — not
/// because the caller was granted anything — and a caller that cannot tell the
/// halves apart will project them alike and disclose the second half's
/// configuration.
///
/// The flag is evaluated by the same access filter the `WHERE` clause is built
/// from, in the same statement, so the two cannot drift.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ListedWorkspace {
    #[sqlx(flatten)]
    pub workspace: Workspace,
    pub readable: bool,
}

/// Patch struct for partial workspace updates. Outer `Option` = "field present
/// in the patch"; inner `Option` = the nullable column value.
#[derive(Default)]
pub struct WorkspacePatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub kind: Option<String>,
    pub scope: Option<Scope>,
    pub owner_user_id: Option<Option<String>>,
    pub docker_image: Option<Option<String>>,
    pub docker_access: Option<Option<bool>>,
    pub sandbox_service_account: Option<Option<String>>,
    pub system_prompt: Option<Option<String>>,
    pub setup_commands: Option<Option<serde_json::Value>>,
    pub env_vars: Option<Option<serde_json::Value>>,
    pub servers: Option<Option<serde_json::Value>>,
    pub resources: Option<Option<serde_json::Value>>,
    pub network_profile: Option<Option<String>>,
    pub source: Option<String>,
    pub source_repository_id: Option<Option<String>>,
    pub source_key: Option<Option<String>>,
    pub source_config_path: Option<Option<String>>,
}

/// DAO trait for workspace persistence.
// reason: `credentials::service` gates its writes on the owning workspace row,
// so its unit tests need a workspace read they can steer.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WorkspaceDao: Send + Sync {
    /// Fetch a single active workspace, or a deleted git-origin tombstone, by
    /// id, filtered by the caller's ACL scope. Returns `None` when the row is
    /// missing, outside the scope, or a plain deleted manual workspace.
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Workspace>, sqlx::Error>;

    /// List active workspaces plus git-origin tombstones visible to the caller,
    /// ordered by name.
    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<Workspace>, sqlx::Error>;

    /// Resolve `(id, name)` for several workspaces in one query. Used to label
    /// rows that already reference a workspace by id, so it is deliberately not
    /// scope-filtered — the caller has already been authorized for the owning
    /// row. Missing or soft-deleted ids are simply absent from the result.
    async fn find_names_by_ids(&self, ids: &[String])
    -> Result<Vec<(String, String)>, sqlx::Error>;

    /// Every non-deleted workspace the caller may read, plus every non-deleted
    /// workspace in `also_include` whether or not they may read it, ordered by
    /// name.
    ///
    /// The widening exists for listings that group tasks by their workspace. A
    /// task the caller may read can run in a workspace they may not — someone
    /// else's user-scoped one — and a grouping that omits the workspace omits
    /// the task with it.
    ///
    /// Unlike [`Self::list`] this excludes git-origin tombstones: a tombstone is
    /// a deleted workspace kept only so a sync will not recreate it, and a
    /// listing that groups live work has nothing to put under it.
    ///
    /// Each row says which half of the union it came from — see
    /// [`ListedWorkspace`]. A caller that projects the two halves the same way
    /// hands the widened half's configuration to someone the read rule refused,
    /// so the distinction is returned rather than left to be re-derived.
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error>;

    /// List a workspace's repo placements, primary first then by `position`.
    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error>;

    /// List repo placements for several workspaces in one query (avoids the
    /// N+1 in list endpoints). Ordered by workspace, then primary/position.
    async fn find_repos_for_workspaces(
        &self,
        workspace_ids: &[String],
    ) -> Result<Vec<WorkspaceRepo>, sqlx::Error>;

    /// The workspace's primary repo placement (the diff/PR/AGENTS.md target),
    /// falling back to the lowest-`position` repo when none is flagged primary.
    async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error>;

    /// Insert a workspace row.
    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error>;

    /// Apply a partial update, scoped to caller control. Returns `false` when
    /// nothing changed, row is missing, or caller cannot control the row.
    async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Soft-delete a workspace, scoped to caller control. Returns `false` when
    /// already deleted, missing, or caller cannot control the row.
    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Soft-delete a git-origin workspace as a manual tombstone. The provenance
    /// key remains occupied so automatic and repo-wide syncs do not recreate it.
    async fn soft_delete_git_origin(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Insert a workspace_repo placement, gated on caller controlling the workspace.
    async fn add_repo(&self, row: &WorkspaceRepo, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Remove a placement from a workspace, gated on caller controlling the workspace.
    /// Returns `false` when not found or caller cannot control the workspace.
    async fn remove_repo(
        &self,
        workspace_id: &str,
        placement_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Make `placement_id` the sole primary placement in the workspace
    /// (transactionally clears any existing primary first). Returns `false`
    /// when the placement does not belong to the workspace or caller cannot control it.
    async fn set_primary(
        &self,
        workspace_id: &str,
        placement_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Recompute and persist the derived `kind` (`blank`/`single`/`multi`) from
    /// the current placement count.
    async fn refresh_kind(&self, workspace_id: &str, ctx: &ScopeContext)
    -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`WorkspaceDao`].
pub struct PgWorkspaceDao(pub DbPool);

#[async_trait]
impl WorkspaceDao for PgWorkspaceDao {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Workspace>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "SELECT * FROM workspaces \
             WHERE id = $1 \
             AND {ACTIVE_OR_GIT_TOMBSTONE} \
             AND {}",
            access.sql()
        );
        sqlx::query_as::<_, Workspace>(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceRepo>(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC"#,
        )
        .bind(workspace_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceRepo>(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC
               LIMIT 1"#,
        )
        .bind(workspace_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO workspaces (
                   id, name, description, kind,
                   docker_image, docker_access, sandbox_service_account,
                   system_prompt, setup_commands, env_vars, servers, resources,
                   network_profile, source, source_repository_id, source_key, source_config_path,
                   scope, owner_user_id, created_by, deleted
               ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                   $13, $14, $15, $16, $17, $18, $19, $20, $21
               )"#,
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.kind)
        .bind(&row.docker_image)
        .bind(row.docker_access)
        .bind(&row.sandbox_service_account)
        .bind(&row.system_prompt)
        .bind(&row.setup_commands)
        .bind(&row.env_vars)
        .bind(&row.servers)
        .bind(&row.resources)
        .bind(&row.network_profile)
        .bind(&row.source)
        .bind(&row.source_repository_id)
        .bind(&row.source_key)
        .bind(&row.source_config_path)
        .bind(row.scope)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.deleted)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<Workspace>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "", 1);
        let sql = format!(
            "SELECT * FROM workspaces \
             WHERE {ACTIVE_OR_GIT_TOMBSTONE} \
             AND {} \
             ORDER BY name ASC",
            access.sql()
        );
        sqlx::query_as::<_, Workspace>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "", 1);
        let ids_param = access.next_param();
        // The same fragment twice, on purpose: as the admitting half of the
        // `WHERE` and as the column that says which half admitted the row. One
        // expression, so a row can never be reported readable by one and not
        // the other.
        let sql = format!(
            "SELECT *, {} AS readable FROM workspaces \
             WHERE (deleted IS NULL OR deleted = false) \
             AND ({} OR id = ANY(${ids_param})) \
             ORDER BY name ASC",
            access.sql(),
            access.sql()
        );
        sqlx::query_as::<_, ListedWorkspace>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .bind(also_include)
            .fetch_all(&self.0)
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
            "SELECT id, name FROM workspaces WHERE id = ANY($1) AND (deleted IS NULL OR deleted = false)",
        )
        .bind(ids)
        .fetch_all(&self.0)
        .await
    }

    async fn find_repos_for_workspaces(
        &self,
        workspace_ids: &[String],
    ) -> Result<Vec<WorkspaceRepo>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceRepo>(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = ANY($1)
               ORDER BY workspace_id ASC, is_primary DESC, position ASC, id ASC"#,
        )
        .bind(workspace_ids)
        .fetch_all(&self.0)
        .await
    }

    async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
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
        push_set!(description);
        push_set!(kind);
        push_set!(scope);
        push_set!(owner_user_id);
        push_set!(docker_image);
        push_set!(docker_access);
        push_set!(sandbox_service_account);
        push_set!(system_prompt);
        push_set!(setup_commands);
        push_set!(env_vars);
        push_set!(servers);
        push_set!(resources);
        push_set!(network_profile);
        push_set!(source);
        push_set!(source_repository_id);
        push_set!(source_key);
        push_set!(source_config_path);

        if sets.is_empty() {
            return Ok(false);
        }

        sets.push(format!("updated_at = ${idx}"));
        idx += 1;

        let control = ctx.control_filter("", idx);
        let sql = format!(
            "UPDATE workspaces SET {} \
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
        bind_opt!(description);
        bind_opt!(kind);
        bind_opt!(scope);
        bind_opt!(owner_user_id);
        bind_opt!(docker_image);
        bind_opt!(docker_access);
        bind_opt!(sandbox_service_account);
        bind_opt!(system_prompt);
        bind_opt!(setup_commands);
        bind_opt!(env_vars);
        bind_opt!(servers);
        bind_opt!(resources);
        bind_opt!(network_profile);
        bind_opt!(source);
        bind_opt!(source_repository_id);
        bind_opt!(source_key);
        bind_opt!(source_config_path);

        q = q.bind(now);
        q = q.bind(ctx.is_admin);
        q = q.bind(&ctx.user_id);

        let result = q.execute(&self.0).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let control = ctx.control_filter("", 3);
        let sql = format!(
            "UPDATE workspaces SET deleted = true, updated_at = $2 \
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

    async fn soft_delete_git_origin(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let control = ctx.control_filter("", 3);
        let sql = format!(
            "UPDATE workspaces \
             SET deleted = true, source = 'manual', updated_at = $2 \
             WHERE id = $1 \
               AND (deleted IS NULL OR deleted = false) \
               AND source_repository_id IS NOT NULL \
               AND source_key IS NOT NULL \
               AND {}",
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

    async fn add_repo(&self, row: &WorkspaceRepo, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        // Gate on caller controlling the workspace via EXISTS subquery.
        let control = ctx.control_filter("w.", 8);
        let sql = format!(
            r#"INSERT INTO workspace_repos (
                   id, workspace_id, repository_id, branch, mount_name, is_primary, position
               )
               SELECT $1, $2, $3, $4, $5, $6, $7
               WHERE EXISTS (
                   SELECT 1 FROM workspaces w
                   WHERE w.id = $2 AND (w.deleted IS NULL OR w.deleted = false) AND {}
               )"#,
            control.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(&row.id)
            .bind(&row.workspace_id)
            .bind(&row.repository_id)
            .bind(&row.branch)
            .bind(&row.mount_name)
            .bind(row.is_primary)
            .bind(row.position)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn remove_repo(
        &self,
        workspace_id: &str,
        placement_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let control = ctx.control_filter("w.", 3);
        let sql = format!(
            "DELETE FROM workspace_repos \
             WHERE id = $1 AND workspace_id = $2 \
             AND EXISTS (SELECT 1 FROM workspaces w WHERE w.id = $2 AND {})",
            control.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(placement_id)
            .bind(workspace_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_primary(
        &self,
        workspace_id: &str,
        placement_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;

        // Check the placement belongs to this workspace AND caller controls it.
        let control = ctx.control_filter("w.", 3);
        let check_sql = format!(
            "SELECT count(*) FROM workspace_repos wr \
             JOIN workspaces w ON w.id = wr.workspace_id \
             WHERE wr.id = $1 AND wr.workspace_id = $2 AND {}",
            control.sql()
        );
        let belongs: i64 = sqlx::query_scalar(AssertSqlSafe(check_sql))
            .bind(placement_id)
            .bind(workspace_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await?;
        if belongs == 0 {
            return Ok(false);
        }

        // Clear the existing primary first so the partial unique index
        // (`workspace_repos_one_primary`) never sees two primaries.
        sqlx::query(
            "UPDATE workspace_repos SET is_primary = false \
             WHERE workspace_id = $1 AND is_primary = true",
        )
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE workspace_repos SET is_primary = true WHERE id = $1")
            .bind(placement_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(true)
    }

    async fn refresh_kind(
        &self,
        workspace_id: &str,
        ctx: &ScopeContext,
    ) -> Result<(), sqlx::Error> {
        let control = ctx.control_filter("", 3);
        let sql = format!(
            "UPDATE workspaces \
             SET kind = CASE (SELECT count(*) FROM workspace_repos WHERE workspace_id = $1) \
                        WHEN 0 THEN 'blank' WHEN 1 THEN 'single' ELSE 'multi' END, \
                 updated_at = $2 \
             WHERE id = $1 AND {}",
            control.sql()
        );
        sqlx::query(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(chrono::Utc::now())
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}

/// Idempotently ensure a repository's deterministic **shadow workspace** exists
/// (`ws_<repository_id>` + its primary placement `wsr_<repository_id>`), and
/// return the workspace id.
///
/// This is the runtime twin of the Phase A migration backfill: it copies the
/// same repository columns 1:1, uses the same `kind = 'single'`, and creates the
/// same single primary placement, so a task created at runtime lands on exactly
/// the shape the migration produced for pre-existing tasks. The workspace insert
/// dedupes on `ON CONFLICT (id)`; the primary placement is skipped when the
/// workspace already has one, so it stays correct even after `git_sync` has
/// re-elected the primary under a random id. The id is returned unconditionally
/// rather than gated on a prior existence `SELECT`. Both inserts run in one
/// transaction.
///
/// If the repository row does not exist the `INSERT ... SELECT`s are no-ops; the
/// returned id is still `ws_<repository_id>` (there is no FK on
/// `tasks.workspace_id` during the additive transition).
pub async fn ensure_shadow_workspace(
    pool: &DbPool,
    repository_id: &str,
) -> Result<String, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"INSERT INTO workspaces (
               id, name, description, kind,
               docker_image, docker_access, sandbox_service_account,
               system_prompt, setup_commands, env_vars, servers, resources,
               scope, owner_user_id, created_by, deleted, created_at, updated_at
           )
           SELECT
               'ws_' || r.id, r.name, r.description, 'single',
               r.docker_image, r.docker_access, r.sandbox_service_account,
               r.system_prompt, r.setup_commands, r.env_vars,
               COALESCE(r.servers, '[]'::jsonb), r.resources,
               COALESCE(r.scope, 'user'), r.owner_user_id, r.created_by, r.deleted,
               COALESCE(r.created_at, now()), COALESCE(r.updated_at, now())
           FROM repositories r
           WHERE r.id = $1
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(repository_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO workspace_repos (
               id, workspace_id, repository_id, branch, mount_name, is_primary, position
           )
           SELECT
               'wsr_' || r.id, 'ws_' || r.id, r.id, NULL, NULL, true, 0
           FROM repositories r
           WHERE r.id = $1
             AND NOT EXISTS (
                 SELECT 1 FROM workspace_repos wr
                 WHERE wr.workspace_id = 'ws_' || r.id AND wr.is_primary
             )
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(repository_id)
    .execute(&mut *tx)
    .await?;

    let workspace_id = format!("ws_{repository_id}");
    tx.commit().await?;
    Ok(workspace_id)
}

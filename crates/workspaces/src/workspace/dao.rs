//! Workspace DAO — queries for workspace + `workspace_repo` persistence.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{
    ListedWorkspace, ListedWorkspaceRow, ScriptConfig, UpdateOutcome, Workspace, WorkspacePatch,
    WorkspaceRepo, WorkspaceRepoRow, WorkspaceRow, WorkspaceScriptsTarget, encode_column,
};
use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};

const ACTIVE_OR_GIT_TOMBSTONE: &str = "((deleted IS NULL OR deleted = false) OR \
    (deleted = true AND source = 'manual' AND source_repository_id IS NOT NULL AND source_key IS NOT NULL))";

/// DAO trait for workspace persistence.
// reason: `credentials::service` gates its writes on the owning workspace row,
// so its unit tests need a workspace read they can steer.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait WorkspaceDao: Send + Sync {
    /// Read an active workspace's scripts for replacement under the caller's
    /// read grant.
    async fn scripts_write_target(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceScriptsTarget>, sqlx::Error>;

    /// Replace scripts only while the workspace remains readable, active and
    /// not Git-owned.
    async fn replace_scripts(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
        scripts: &[ScriptConfig],
    ) -> Result<bool, sqlx::Error>;
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

    /// Resolve `(id, name)` for several workspaces in one query, to label rows
    /// that already reference a workspace by id — deliberately not
    /// scope-filtered since the caller was already authorized for the owning
    /// row; missing or soft-deleted ids are simply absent.
    async fn find_names_by_ids(&self, ids: &[String])
    -> Result<Vec<(String, String)>, sqlx::Error>;

    /// Every non-deleted workspace the caller may read, plus every workspace in
    /// `also_include` regardless of readability or deletion, for listings that
    /// group tasks by workspace.
    ///
    /// Each row says which half of the union it came from ([`ListedWorkspace`])
    /// — projecting both halves the same way would leak the widened half's
    /// configuration to someone the read rule refused.
    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error>;

    /// Every live workspace placing this repository, primary placements first
    /// then oldest first.
    ///
    /// Not scope-filtered, and tombstones excluded: this answers "where can
    /// work on this repository run", which no deleted row can.
    async fn find_by_repository(
        &self,
        repository_id: &str,
    ) -> Result<Vec<Workspace>, sqlx::Error>;

    /// List a workspace's repo placements, primary first then by `position`.
    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error>;

    /// List repo placements for several workspaces in one query (avoids the N+1
    /// in list endpoints). Ordered by workspace, then primary/position.
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

    /// Apply a partial update, scoped to caller control.
    async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
        ctx: &ScopeContext,
    ) -> Result<UpdateOutcome, sqlx::Error>;

    /// Soft-delete a workspace, scoped to caller control. Returns `false` when
    /// already deleted, missing, or caller cannot control the row.
    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Soft-delete a git-origin workspace as a manual tombstone. The provenance
    /// key remains occupied so automatic and repo-wide syncs do not recreate
    /// it.
    async fn soft_delete_git_origin(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Insert a `workspace_repo` placement, gated on caller controlling the
    /// workspace.
    async fn add_repo(&self, row: &WorkspaceRepo, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Remove a placement from a workspace, gated on caller controlling the
    /// workspace. Returns `false` when not found or caller cannot control the
    /// workspace.
    async fn remove_repo(
        &self,
        workspace_id: &str,
        placement_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error>;

    /// Make `placement_id` the sole primary placement in the workspace
    /// (transactionally clears any existing primary first). Returns `false`
    /// when the placement does not belong to the workspace or caller cannot
    /// control it.
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
pub(crate) struct PgWorkspaceDao(pub(crate) DbPool);

#[async_trait]
impl WorkspaceDao for PgWorkspaceDao {
    async fn scripts_write_target(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceScriptsTarget>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "SELECT source, COALESCE(scripts, '[]'::jsonb) FROM workspaces WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND {}",
            access.sql()
        );
        let row = sqlx::query_as::<_, (String, serde_json::Value)>(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_optional(&self.0)
            .await?;
        Ok(row.map(|(source, scripts)| WorkspaceScriptsTarget {
            source,
            // An unreadable value loses hidden-field carry-over so a caller can
            // replace corrupt configuration with a readable list.
            scripts: serde_json::from_value(scripts).unwrap_or_default(),
        }))
    }

    async fn replace_scripts(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        scripts: &[ScriptConfig],
    ) -> Result<bool, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 3);
        let sql = format!(
            // Access and source can change after scripts_write_target; recheck
            // both in this statement to prevent a write outside the caller's scope.
            "UPDATE workspaces SET scripts = $2, updated_at = NOW() WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND source <> 'git' AND {}",
            access.sql()
        );
        let scripts = serde_json::to_value(scripts).unwrap_or_else(|_| serde_json::json!([]));
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(scripts)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Workspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "SELECT * FROM workspaces \
             WHERE id = $1 \
             AND {ACTIVE_OR_GIT_TOMBSTONE} \
             AND {}",
            access.sql()
        );
        let row: Option<WorkspaceRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await?;
        row.map(Workspace::try_from).transpose()
    }

    async fn find_by_repository(
        &self,
        repository_id: &str,
    ) -> Result<Vec<Workspace>, sqlx::Error> {
        let rows: Vec<WorkspaceRow> = sqlx::query_as(
            // Grouped, not a plain join: only `mount_name` and `is_primary` are
            // unique per workspace, so a workspace placing this repository at
            // two mounts would otherwise be returned twice.
            r#"SELECT w.* FROM workspaces w
               JOIN workspace_repos r ON r.workspace_id = w.id
               WHERE r.repository_id = $1
                 AND (w.deleted IS NULL OR w.deleted = false)
               GROUP BY w.id
               ORDER BY bool_or(r.is_primary) DESC, w.created_at ASC, w.id ASC"#,
        )
        .bind(repository_id)
        .fetch_all(&self.0)
        .await?;
        rows.into_iter().map(Workspace::try_from).collect()
    }

    async fn find_repos(&self, workspace_id: &str) -> Result<Vec<WorkspaceRepo>, sqlx::Error> {
        let rows: Vec<WorkspaceRepoRow> = sqlx::query_as(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC"#,
        )
        .bind(workspace_id)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(WorkspaceRepo::from).collect())
    }

    async fn find_primary_repo(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceRepo>, sqlx::Error> {
        let row: Option<WorkspaceRepoRow> = sqlx::query_as(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = $1
               ORDER BY is_primary DESC, position ASC, id ASC
               LIMIT 1"#,
        )
        .bind(workspace_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(WorkspaceRepo::from))
    }

    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO workspaces (
                   id, name, description, icon, kind,
                   docker_image, docker_access, sandbox_service_account,
                   system_prompt, setup_commands, env_vars, servers, scripts, resources,
                   network_profile, node_selector, default_model, default_effort,
                   source, source_repository_id, source_key, source_config_path,
                   scope, owner_user_id, run_as_user_id, created_by, deleted
               ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                   $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27
               )"#,
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(encode_column(row.icon.as_ref(), "icon")?)
        .bind(row.kind.as_str())
        .bind(&row.docker_image)
        .bind(row.docker_access)
        .bind(&row.sandbox_service_account)
        .bind(&row.system_prompt)
        .bind(encode_column(
            row.setup_commands.as_ref(),
            "setup_commands",
        )?)
        .bind(encode_column(row.env_vars.as_ref(), "env_vars")?)
        .bind(encode_column(row.servers.as_ref(), "servers")?)
        .bind(encode_column(row.scripts.as_ref(), "scripts")?)
        .bind(encode_column(row.resources.as_ref(), "resources")?)
        .bind(&row.network_profile)
        .bind(encode_column(row.node_selector.as_ref(), "node_selector")?)
        .bind(&row.default_model)
        .bind(&row.default_effort)
        .bind(row.source.as_str())
        .bind(&row.source_repository_id)
        .bind(&row.source_key)
        .bind(&row.source_config_path)
        .bind(row.scope.as_str())
        .bind(&row.owner_user_id)
        .bind(&row.run_as_user_id)
        .bind(&row.created_by)
        .bind(row.deleted)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<Workspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
        let sql = format!(
            "SELECT * FROM workspaces \
             WHERE {ACTIVE_OR_GIT_TOMBSTONE} \
             AND {} \
             ORDER BY name ASC",
            access.sql()
        );
        let rows: Vec<WorkspaceRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await?;
        rows.into_iter().map(Workspace::try_from).collect()
    }

    async fn list_readable_including(
        &self,
        ctx: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<ListedWorkspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
        let ids_param = access.next_param();
        // The same fragment twice, on purpose: as the admitting half of the
        // `WHERE` and as the column that says which half admitted the row. One
        // expression, so a row can never be reported readable by one and not
        // the other.
        //
        // `deleted` narrows the readable half only: a workspace named by a task
        // the caller may read is listed even once a sync has tombstoned it.
        let sql = format!(
            "SELECT *, {} AS readable FROM workspaces \
             WHERE ((deleted IS NULL OR deleted = false) AND {}) \
             OR id = ANY(${ids_param}) \
             ORDER BY name ASC",
            access.sql(),
            access.sql()
        );
        let rows: Vec<ListedWorkspaceRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .bind(also_include)
            .fetch_all(&self.0)
            .await?;
        rows.into_iter().map(ListedWorkspace::try_from).collect()
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
        let rows: Vec<WorkspaceRepoRow> = sqlx::query_as(
            r#"SELECT * FROM workspace_repos
               WHERE workspace_id = ANY($1)
               ORDER BY workspace_id ASC, is_primary DESC, position ASC, id ASC"#,
        )
        .bind(workspace_ids)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(WorkspaceRepo::from).collect())
    }

    async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
        ctx: &ScopeContext,
    ) -> Result<UpdateOutcome, sqlx::Error> {
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
        push_set!(icon);
        push_set!(kind);
        push_set!(scope);
        push_set!(owner_user_id);
        push_set!(run_as_user_id);
        push_set!(docker_image);
        push_set!(docker_access);
        push_set!(sandbox_service_account);
        push_set!(system_prompt);
        push_set!(setup_commands);
        push_set!(env_vars);
        push_set!(servers);
        push_set!(scripts);
        push_set!(resources);
        push_set!(network_profile);
        push_set!(node_selector);
        push_set!(default_model);
        push_set!(default_effort);
        push_set!(source);
        push_set!(source_repository_id);
        push_set!(source_key);
        push_set!(source_config_path);

        if sets.is_empty() {
            return Ok(UpdateOutcome::NoFields);
        }

        sets.push(format!("updated_at = ${idx}"));
        idx += 1;

        let control = moatless_core::db::access_control::write_filter("", idx);
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

        macro_rules! bind_json {
            ($field:ident) => {
                if let Some(ref val) = patch.$field {
                    q = q.bind(encode_column(val.as_ref(), stringify!($field))?);
                }
            };
        }

        bind_opt!(name);
        bind_opt!(description);
        bind_json!(icon);
        if let Some(kind) = patch.kind {
            q = q.bind(kind.as_str());
        }
        if let Some(scope) = patch.scope {
            q = q.bind(scope.as_str());
        }
        bind_opt!(owner_user_id);
        bind_opt!(run_as_user_id);
        bind_opt!(docker_image);
        bind_opt!(docker_access);
        bind_opt!(sandbox_service_account);
        bind_opt!(system_prompt);
        bind_json!(setup_commands);
        bind_json!(env_vars);
        bind_json!(servers);
        bind_json!(scripts);
        bind_json!(resources);
        bind_opt!(network_profile);
        bind_json!(node_selector);
        bind_opt!(default_model);
        bind_opt!(default_effort);
        if let Some(source) = patch.source {
            q = q.bind(source.as_str());
        }
        bind_opt!(source_repository_id);
        bind_opt!(source_key);
        bind_opt!(source_config_path);

        q = q.bind(now);
        q = q.bind(ctx.is_admin);
        q = q.bind(&ctx.user_id);

        let result = q.execute(&self.0).await?;
        if result.rows_affected() > 0 {
            Ok(UpdateOutcome::Applied)
        } else {
            Ok(UpdateOutcome::NoMatch)
        }
    }

    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let control = moatless_core::db::access_control::write_filter("", 3);
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
        let control = moatless_core::db::access_control::write_filter("", 3);
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
        let control = moatless_core::db::access_control::write_filter("w.", 8);
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
        let control = moatless_core::db::access_control::write_filter("w.", 3);
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
        let control = moatless_core::db::access_control::write_filter("w.", 3);
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
        let control = moatless_core::db::access_control::write_filter("", 3);
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

/// Soft-delete the Git workspaces declared by a deleted repository.
pub(crate) async fn delete_declared_workspaces(
    db: &moatless_core::db::DbPool,
    repository_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE workspaces SET deleted = true, updated_at = now() \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false)",
    )
    .bind(repository_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Read the active Git source keys reconciled for a repository.
pub(crate) async fn git_source_keys(
    db: &DbPool,
    repository_id: &str,
) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, source_key FROM workspaces \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false)",
    )
    .bind(repository_id)
    .fetch_all(db)
    .await
}

/// Find a Git source's workspace, including manual overrides and tombstones.
pub(crate) async fn workspace_for_git_source(
    db: &DbPool,
    repository_id: &str,
    source_key: &str,
) -> Result<Option<(String, String, Option<bool>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, source, deleted FROM workspaces \
             WHERE source_repository_id = $1 AND source_key = $2 \
               AND source IN ('git', 'manual') \
             LIMIT 1",
    )
    .bind(repository_id)
    .bind(source_key)
    .fetch_optional(db)
    .await
}

/// Restore a workspace's tombstone before applying Git content.
pub(crate) async fn restore_git_workspace(
    db: &DbPool,
    workspace_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspaces SET deleted = false WHERE id = $1 AND deleted = true")
        .bind(workspace_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Atomically replace Git placements and derive the workspace kind.
pub(crate) async fn replace_git_placements(
    db: &DbPool,
    workspace_id: &str,
    placements: &[WorkspaceRepo],
    primary: Option<&str>,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("SELECT id FROM workspaces WHERE id = $1 FOR UPDATE")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM workspace_repos WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;
    for row in placements {
        sqlx::query("INSERT INTO workspace_repos (id, workspace_id, repository_id, branch, mount_name, is_primary, position) SELECT $1, $2, $3, $4, $5, false, $6 WHERE EXISTS (SELECT 1 FROM workspaces WHERE id = $2 AND (deleted IS NULL OR deleted = false))")
            .bind(&row.id).bind(workspace_id).bind(&row.repository_id).bind(&row.branch).bind(&row.mount_name).bind(row.position)
            .execute(&mut *tx).await?;
    }
    if let Some(primary) = primary {
        sqlx::query(
            "UPDATE workspace_repos SET is_primary = true WHERE id = $1 AND workspace_id = $2",
        )
        .bind(primary)
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE workspaces SET kind = CASE (SELECT count(*) FROM workspace_repos WHERE workspace_id = $1) WHEN 0 THEN 'blank' WHEN 1 THEN 'single' ELSE 'multi' END, updated_at = now() WHERE id = $1")
        .bind(workspace_id).execute(&mut *tx).await?;
    tx.commit().await
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod db_tests;


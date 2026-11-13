//! Workspace DAO — queries for workspace + `workspace_repo` persistence.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{
    ListedWorkspace, ListedWorkspaceRow, ScriptConfig, UpdateOutcome, Workspace, WorkspacePatch,
    WorkspaceRepo, WorkspaceRepoRow, WorkspaceRow, encode_column,
};
use moatless_core::change_feed::listing;
use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};

/// The columns a T3 project row or the Workspace read rule is built from. A
/// change to one is a listing change; a change to any other is not.
const PROJECTED_COLUMNS: &[&str] = &[
    "name",
    "icon",
    "scope",
    "owner_user_id",
    "default_model",
    "default_effort",
    "scripts",
    "source",
];

const ACTIVE_OR_GIT_TOMBSTONE: &str = "((deleted IS NULL OR deleted = false) OR \
    (deleted = true AND source = 'manual' AND source_repository_id IS NOT NULL AND source_key IS NOT NULL))";

// A wildcard changes cached result metadata when another replica adds an
// unrelated column, breaking shell and thread snapshots on surviving readers.
const WORKSPACE_COLUMNS: &str = "w.id, w.name, w.description, w.icon, w.kind,
    w.docker_image, w.docker_access, w.sandbox_service_account, w.system_prompt,
    w.setup_commands, w.env_vars, w.scripts, w.resources, w.language_servers,
    w.network_profile, w.node_selector, w.services, w.default_model, w.default_effort,
    w.source, w.source_repository_id, w.source_key, w.source_config_path,
    w.scope, w.owner_user_id, w.run_as_user_id, w.created_by, w.deleted,
    w.created_at, w.updated_at";

/// DAO trait for workspace persistence.
// reason: `credentials::service` gates its writes on the owning workspace row,
// so its unit tests need a workspace read they can steer.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait WorkspaceDao: Send + Sync {
    /// A live Workspace's Scripts, unscoped: `None` when it is gone or
    /// deleted. A stored value that will not decode reads as no Scripts.
    ///
    /// The caller must already have authorized the read through a Task that
    /// runs in this Workspace; nothing here checks the viewer.
    async fn scripts_unscoped(
        &self,
        workspace_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error>;

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

    /// Every non-deleted workspace the caller may read, plus every workspace
    /// in `also_include` regardless of readability, for listings that group
    /// tasks by workspace.
    ///
    /// Each row says which half admitted it ([`ListedWorkspace`]): projecting
    /// both the same way would leak the widened half's config.
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
    async fn find_by_repository(&self, repository_id: &str) -> Result<Vec<Workspace>, sqlx::Error>;

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

    /// The repository id of [`Self::find_primary_repo`]'s placement, read only
    /// when the caller may read the live Workspace.
    ///
    /// `None` when the Workspace is unreadable, deleted, missing or places no
    /// repository: one statement, so the scope check and the pick cannot drift.
    async fn readable_primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    /// Insert a workspace row, recording a listing change.
    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error>;

    /// Apply a partial update, scoped to caller control, recording a listing
    /// change when a [`PROJECTED_COLUMNS`] value moved.
    async fn update(
        &self,
        id: &str,
        patch: &WorkspacePatch,
        ctx: &ScopeContext,
    ) -> Result<UpdateOutcome, sqlx::Error>;

    /// Soft-delete a workspace, scoped to caller control, recording a listing
    /// change. Returns `false` when already deleted, missing, or caller cannot
    /// control the row.
    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    /// Insert a `workspace_repo` placement, gated on caller controlling the
    /// workspace.
    #[allow(
        dead_code,
        reason = "only `Workspaces::add_repo`, built under `test-support`, calls it"
    )]
    async fn add_repo(&self, row: &WorkspaceRepo, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`WorkspaceDao`].
pub(crate) struct PgWorkspaceDao(pub(crate) DbPool);

#[async_trait]
impl WorkspaceDao for PgWorkspaceDao {
    async fn scripts_unscoped(
        &self,
        workspace_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error> {
        // `COALESCE` so a workspace predating the scripts column reads as an
        // empty list rather than a NULL that fails to decode.
        let raw = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(scripts, '[]'::jsonb) FROM workspaces \
             WHERE id = $1 AND (deleted IS NULL OR deleted = false)",
        )
        .bind(workspace_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(raw.map(|raw| serde_json::from_value(raw).unwrap_or_default()))
    }

    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<Workspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "SELECT {WORKSPACE_COLUMNS} FROM workspaces w \
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

    async fn find_by_repository(&self, repository_id: &str) -> Result<Vec<Workspace>, sqlx::Error> {
        let rows: Vec<WorkspaceRow> = sqlx::query_as(AssertSqlSafe(format!(
            // Grouped, not a plain join: only `mount_name` and `is_primary` are
            // unique per workspace, so a workspace placing this repository at
            // two mounts would otherwise be returned twice.
            r#"SELECT {WORKSPACE_COLUMNS} FROM workspaces w
               JOIN workspace_repos r ON r.workspace_id = w.id
               WHERE r.repository_id = $1
                 AND (w.deleted IS NULL OR w.deleted = false)
               GROUP BY w.id
               ORDER BY bool_or(r.is_primary) DESC, w.created_at ASC, w.id ASC"#,
        )))
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

    async fn readable_primary_repository(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "w.", 2);
        let sql = format!(
            "SELECT r.repository_id FROM workspace_repos r \
             JOIN workspaces w ON w.id = r.workspace_id \
             WHERE r.workspace_id = $1 \
             AND (w.deleted IS NULL OR w.deleted = false) \
             AND {} \
             ORDER BY r.is_primary DESC, r.position ASC, r.id ASC \
             LIMIT 1",
            access.sql()
        );
        sqlx::query_scalar::<_, String>(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn insert_workspace(&self, row: &Workspace) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        sqlx::query(
            r#"INSERT INTO workspaces (
                   id, name, description, icon, kind,
                   docker_image, docker_access, sandbox_service_account,
                   system_prompt, setup_commands, env_vars, scripts, resources,
                   language_servers, network_profile, node_selector, default_model,
                   default_effort, source, source_repository_id, source_key,
                   source_config_path, scope, owner_user_id, run_as_user_id,
                   created_by, deleted, services
               ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                   $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26,
                   $27, $28
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
        .bind(encode_column(row.scripts.as_ref(), "scripts")?)
        .bind(encode_column(row.resources.as_ref(), "resources")?)
        .bind(encode_column(
            row.language_servers.as_ref(),
            "language_servers",
        )?)
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
        .bind(encode_column(row.services.as_ref(), "services")?)
        .execute(&mut *tx)
        .await?;
        // A new project is a listing change before any thread runs in it: a
        // resumed client otherwise never learns it exists.
        listing::record_workspace_change(&mut tx, &row.id).await?;
        tx.commit().await
    }

    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<Workspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
        let sql = format!(
            "SELECT {WORKSPACE_COLUMNS} FROM workspaces w \
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
        // The same fragment twice, deliberately: as the admitting half of the
        // `WHERE` and as the column naming which half admitted the row — one
        // expression, so a row can't be readable by one and not the other.
        //
        // `deleted` narrows only the readable half: a workspace named by a
        // readable task still lists even once a sync has tombstoned it.
        let sql = format!(
            "SELECT {WORKSPACE_COLUMNS}, {} AS readable FROM workspaces w \
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
        push_set!(scripts);
        push_set!(resources);
        push_set!(language_servers);
        push_set!(network_profile);
        push_set!(node_selector);
        push_set!(services);
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

        let control =
            moatless_core::db::access_control::write_filter(ScopedEntity::Resource, "", idx);
        // `before` is aliased column by column so the unqualified names in the
        // `SET` and the write filter still resolve to `workspaces` alone.
        let before_columns = PROJECTED_COLUMNS
            .iter()
            .map(|column| format!("{column} AS before_{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        let projected_before = PROJECTED_COLUMNS
            .iter()
            .map(|column| format!("before.before_{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        let projected_after = PROJECTED_COLUMNS
            .iter()
            .map(|column| format!("workspaces.{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE workspaces SET {} \
             FROM (SELECT {before_columns} FROM workspaces WHERE id = $1 FOR UPDATE) AS before \
             WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND {} \
             RETURNING ({projected_before}) IS DISTINCT FROM ({projected_after})",
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
        bind_json!(scripts);
        bind_json!(resources);
        bind_json!(language_servers);
        bind_opt!(network_profile);
        bind_json!(node_selector);
        bind_json!(services);
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

        let mut tx = self.0.begin().await?;
        let Some(row) = q.fetch_optional(&mut *tx).await? else {
            return Ok(UpdateOutcome::NoMatch);
        };
        let projected_changed: bool = sqlx::Row::try_get(&row, 0)?;
        // Only a change the project row shows: git sync re-applies every
        // Workspace on each sync, and recording those would replay every
        // project to every client.
        if projected_changed {
            listing::record_workspace_change(&mut tx, id).await?;
        }
        tx.commit().await?;
        Ok(UpdateOutcome::Applied)
    }

    async fn soft_delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let control =
            moatless_core::db::access_control::write_filter(ScopedEntity::Resource, "", 3);
        let sql = format!(
            "UPDATE workspaces SET deleted = true, updated_at = $2 \
             WHERE id = $1 AND (deleted IS NULL OR deleted = false) AND {}",
            control.sql()
        );
        let mut tx = self.0.begin().await?;
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(chrono::Utc::now())
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        let deleted = result.rows_affected() > 0;
        if deleted {
            listing::record_workspace_change(&mut tx, id).await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    async fn add_repo(&self, row: &WorkspaceRepo, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        // Gate on caller controlling the workspace via EXISTS subquery.
        let control =
            moatless_core::db::access_control::write_filter(ScopedEntity::Resource, "w.", 8);
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
}

/// Soft-delete the Git workspaces declared by a deleted repository, recording a
/// listing change for each.
pub(crate) async fn delete_declared_workspaces(
    db: &moatless_core::db::DbPool,
    repository_id: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    let deleted: Vec<String> = sqlx::query_scalar(
        "UPDATE workspaces SET deleted = true, updated_at = now() \
         WHERE source_repository_id = $1 AND source = 'git' \
           AND (deleted IS NULL OR deleted = false) \
         RETURNING id",
    )
    .bind(repository_id)
    .fetch_all(&mut *tx)
    .await?;
    for workspace_id in &deleted {
        listing::record_workspace_change(&mut tx, workspace_id).await?;
    }
    tx.commit().await
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

/// The id and `source` of a Git source's workspace, including a legacy manual override and
/// a tombstone.
///
/// The `git` row wins over a manual one sharing the key: reclaiming the manual
/// row instead would violate `workspaces_git_source_unique`.
pub(crate) async fn workspace_for_git_source(
    db: &DbPool,
    repository_id: &str,
    source_key: &str,
) -> Result<Option<(String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, source FROM workspaces \
             WHERE source_repository_id = $1 AND source_key = $2 \
               AND source IN ('git', 'manual') \
             ORDER BY (source = 'git') DESC, (deleted IS TRUE) ASC \
             LIMIT 1",
    )
    .bind(repository_id)
    .bind(source_key)
    .fetch_optional(db)
    .await
}

/// Restore a workspace's tombstone before applying Git content, recording a
/// listing change when it was one.
pub(crate) async fn restore_git_workspace(
    db: &DbPool,
    workspace_id: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    let restored =
        sqlx::query("UPDATE workspaces SET deleted = false WHERE id = $1 AND deleted = true")
            .bind(workspace_id)
            .execute(&mut *tx)
            .await?;
    if restored.rows_affected() > 0 {
        listing::record_workspace_change(&mut tx, workspace_id).await?;
    }
    tx.commit().await
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

/// Take the session advisory lock serializing loop workspace provisioning for one
/// repository URL, on a connection detached from the pool.
///
/// Detached deliberately: dropped on a panic it closes, which releases the
/// lock, where a pooled one would return to the pool still holding it.
pub(crate) async fn lock_workspace_provision(
    db: &DbPool,
    url_key: &str,
) -> Result<sqlx::PgConnection, sqlx::Error> {
    let mut lock = db.acquire().await?.detach();
    sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
        .bind(format!("workspace-provision:{url_key}"))
        .execute(&mut lock)
        .await?;
    Ok(lock)
}

/// Release [`lock_workspace_provision`]'s lock and close its connection.
pub(crate) async fn unlock_workspace_provision(
    mut lock: sqlx::PgConnection,
    url_key: &str,
) -> Result<(), sqlx::Error> {
    let unlocked = sqlx::query("SELECT pg_advisory_unlock(hashtext($1))")
        .bind(format!("workspace-provision:{url_key}"))
        .execute(&mut lock)
        .await;
    let _ = sqlx::Connection::close(lock).await;
    unlocked.map(drop)
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod db_tests;

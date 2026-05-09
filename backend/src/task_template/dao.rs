//! Task template DAO — queries for task template data access.

use async_trait::async_trait;

use super::models::TaskTemplate;
use crate::db::DbPool;
use crate::scope::{Scope, ScopeContext, ScopedEntity};

/// Patch struct for partial updates.
#[derive(Default)]
pub struct TaskTemplatePatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub repository_id: Option<String>,
    pub agent_type: Option<String>,
    pub agent_mode: Option<Option<String>>,
    pub default_model: Option<Option<String>>,
    pub initial_prompt: Option<Option<String>>,
    pub skills: Option<Option<serde_json::Value>>,
    pub scope: Option<Scope>,
    pub team_id: Option<Option<String>>,
    pub featured: Option<bool>,
    pub display_order: Option<i32>,
}

/// Repository trait for task template persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskTemplateRepository: Send + Sync {
    /// Find by id without access control (internal use).
    async fn find_by_id_internal(
        &self,
        id: &str,
    ) -> Result<Option<TaskTemplate>, sqlx::Error>;

    /// Find by id, scoped to access control.
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskTemplate>, sqlx::Error>;

    /// List all templates visible to the user.
    async fn list_for_user(
        &self,
        ctx: &ScopeContext,
    ) -> Result<Vec<TaskTemplate>, sqlx::Error>;

    /// List templates for the chip strip: recent-used first, then featured,
    /// capped at `limit`. User-scoped by `user_id` for the recency join.
    async fn list_featured_for_chips(
        &self,
        ctx: &ScopeContext,
        limit: i64,
    ) -> Result<Vec<TaskTemplate>, sqlx::Error>;

    /// Insert a new template.
    async fn insert(&self, row: &TaskTemplate) -> Result<(), sqlx::Error>;

    /// Update template fields partially.
    async fn update(&self, id: &str, patch: &TaskTemplatePatch) -> Result<bool, sqlx::Error>;

    /// Hard-delete a template.
    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskTemplateRepository`].
pub struct PgTaskTemplateRepository(pub DbPool);

#[async_trait]
impl TaskTemplateRepository for PgTaskTemplateRepository {
    async fn find_by_id_internal(
        &self,
        id: &str,
    ) -> Result<Option<TaskTemplate>, sqlx::Error> {
        sqlx::query_as::<_, TaskTemplate>(
            "SELECT id, name, description, icon, color, repository_id, agent_type, agent_mode, \
             default_model, initial_prompt, skills, scope, team_id, featured, display_order, \
             created_by, created_at, updated_at \
             FROM task_templates WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<TaskTemplate>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Integration, "", 2);
        let sql = format!(
            "SELECT id, name, description, icon, color, repository_id, agent_type, agent_mode, \
             default_model, initial_prompt, skills, scope, team_id, featured, display_order, \
             created_by, created_at, updated_at \
             FROM task_templates WHERE id = $1 AND {}",
            access.sql()
        );
        sqlx::query_as::<_, TaskTemplate>(&sql)
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_for_user(
        &self,
        ctx: &ScopeContext,
    ) -> Result<Vec<TaskTemplate>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Integration, "", 1);
        let sql = format!(
            "SELECT id, name, description, icon, color, repository_id, agent_type, agent_mode, \
             default_model, initial_prompt, skills, scope, team_id, featured, display_order, \
             created_by, created_at, updated_at \
             FROM task_templates WHERE {} \
             ORDER BY display_order ASC, name ASC",
            access.sql()
        );
        sqlx::query_as::<_, TaskTemplate>(&sql)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    async fn list_featured_for_chips(
        &self,
        ctx: &ScopeContext,
        limit: i64,
    ) -> Result<Vec<TaskTemplate>, sqlx::Error> {
        // Server-side merge: user's recent-used first (JOIN tasks), then featured,
        // deduped, capped at limit. Leak-check: tasks join filters by created_by = user_id.
        let access = ctx.access_filter(ScopedEntity::Integration, "tt.", 2);
        let sql = format!(
            "SELECT tt.id, tt.name, tt.description, tt.icon, tt.color, tt.repository_id, \
             tt.agent_type, tt.agent_mode, tt.default_model, tt.initial_prompt, tt.skills, \
             tt.scope, tt.team_id, tt.featured, tt.display_order, tt.created_by, \
             tt.created_at, tt.updated_at \
             FROM task_templates tt \
             LEFT JOIN ( \
               SELECT task_template_id, MAX(created_at_db) AS last_used \
               FROM tasks \
               WHERE task_template_id IS NOT NULL AND created_by = $1 \
               GROUP BY task_template_id \
             ) ru ON ru.task_template_id = tt.id \
             WHERE {} \
             ORDER BY \
               CASE WHEN ru.last_used IS NOT NULL THEN 0 ELSE 1 END ASC, \
               ru.last_used DESC NULLS LAST, \
               tt.featured DESC, \
               tt.display_order ASC \
             LIMIT $5",
            access.sql()
        );
        sqlx::query_as::<_, TaskTemplate>(&sql)
            .bind(&ctx.user_id)   // $1  — recent-used filter
            .bind(ctx.is_admin)   // $2  — access_filter $1
            .bind(&ctx.team_ids)  // $3  — access_filter $2
            .bind(&ctx.user_id)   // $4  — access_filter $3
            .bind(limit)          // $5  — LIMIT
            .fetch_all(&self.0)
            .await
    }

    async fn insert(&self, row: &TaskTemplate) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO task_templates (
                id, name, description, icon, color, repository_id,
                agent_type, agent_mode, default_model, initial_prompt, skills,
                scope, team_id, featured, display_order, created_by, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11,
                $12, $13, $14, $15, $16, $17, $18
            )",
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.icon)
        .bind(&row.color)
        .bind(&row.repository_id)
        .bind(&row.agent_type)
        .bind(&row.agent_mode)
        .bind(&row.default_model)
        .bind(&row.initial_prompt)
        .bind(&row.skills)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(row.featured)
        .bind(row.display_order)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn update(&self, id: &str, patch: &TaskTemplatePatch) -> Result<bool, sqlx::Error> {
        let mut sets: Vec<String> = Vec::new();
        let mut idx = 2usize; // $1 is id

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
        push_set!(color);
        push_set!(repository_id);
        push_set!(agent_type);
        push_set!(agent_mode);
        push_set!(default_model);
        push_set!(initial_prompt);
        push_set!(skills);
        push_set!(scope);
        push_set!(team_id);
        push_set!(featured);
        push_set!(display_order);

        if sets.is_empty() {
            return Ok(false);
        }

        sets.push(format!("updated_at = ${idx}"));

        let sql = format!(
            "UPDATE task_templates SET {} WHERE id = $1",
            sets.join(", ")
        );

        let now = chrono::Utc::now();
        let mut q = sqlx::query(&sql).bind(id);

        macro_rules! bind_opt {
            ($field:ident) => {
                if let Some(ref val) = patch.$field {
                    q = q.bind(val);
                }
            };
        }

        bind_opt!(name);
        bind_opt!(description);
        bind_opt!(icon);
        bind_opt!(color);
        bind_opt!(repository_id);
        bind_opt!(agent_type);
        bind_opt!(agent_mode);
        bind_opt!(default_model);
        bind_opt!(initial_prompt);
        bind_opt!(skills);
        bind_opt!(scope);
        bind_opt!(team_id);
        bind_opt!(featured);
        bind_opt!(display_order);

        q = q.bind(now);

        let result = q.execute(&self.0).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM task_templates WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

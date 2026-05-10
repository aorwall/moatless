//! Persistence for `task_routes`.

use crate::db::DbPool;
use crate::scope::{ScopeContext, ScopedEntity};

use super::super::models::TaskRoute;

pub struct PgTaskRouteRepository(pub DbPool);

#[async_trait::async_trait]
pub trait TaskRouteRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskRoute>, sqlx::Error>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        repository_id: Option<&str>,
    ) -> Result<Vec<TaskRoute>, sqlx::Error>;

    async fn insert(&self, row: NewTaskRoute) -> Result<TaskRoute, sqlx::Error>;

    async fn update(&self, row: UpdateTaskRouteRow) -> Result<TaskRoute, sqlx::Error>;
}

pub struct NewTaskRoute {
    pub id: String,
    pub repository_id: String,
    pub routing_mode: super::super::models::RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct UpdateTaskRouteRow {
    pub id: String,
    pub repository_id: String,
    pub routing_mode: super::super::models::RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
}

impl PgTaskRouteRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskRoute>("SELECT * FROM task_routes WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        repository_id: Option<&str>,
    ) -> Result<Vec<TaskRoute>, sqlx::Error> {
        if let Some(repository_id) = repository_id {
            let access = ctx.access_filter(ScopedEntity::Integration, "", 2);
            let sql = format!(
                "SELECT * FROM task_routes \
                 WHERE repository_id = $1 AND {} \
                 ORDER BY created_at DESC",
                access.sql()
            );
            return sqlx::query_as::<_, TaskRoute>(&sql)
                .bind(repository_id)
                .bind(ctx.is_admin)
                .bind(&ctx.team_ids)
                .bind(&ctx.user_id)
                .fetch_all(&self.0)
                .await;
        }

        let access = ctx.access_filter(ScopedEntity::Integration, "", 1);
        let sql = format!(
            "SELECT * FROM task_routes \
             WHERE {} \
             ORDER BY created_at DESC",
            access.sql()
        );
        sqlx::query_as::<_, TaskRoute>(&sql)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    pub async fn insert(&self, row: NewTaskRoute) -> Result<TaskRoute, sqlx::Error> {
        sqlx::query_as::<_, TaskRoute>(
            r#"INSERT INTO task_routes
               (id, repository_id, routing_mode, ongoing_task_id, prompt_template,
                task_name, tag_ids, run_as_user_id, agent_type, scope, team_id,
                owner_user_id, created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.repository_id)
        .bind(row.routing_mode)
        .bind(row.ongoing_task_id)
        .bind(row.prompt_template)
        .bind(row.task_name)
        .bind(row.tag_ids)
        .bind(row.run_as_user_id)
        .bind(row.agent_type)
        .bind(row.scope)
        .bind(row.team_id)
        .bind(row.owner_user_id)
        .bind(row.created_by)
        .bind(row.created_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn update(&self, row: UpdateTaskRouteRow) -> Result<TaskRoute, sqlx::Error> {
        sqlx::query_as::<_, TaskRoute>(
            r#"UPDATE task_routes
               SET repository_id = $2,
                   routing_mode = $3,
                   ongoing_task_id = $4,
                   prompt_template = $5,
                   task_name = $6,
                   tag_ids = $7,
                   run_as_user_id = $8,
                   agent_type = $9,
                   scope = $10,
                   team_id = $11,
                   owner_user_id = $12,
                   updated_at = now()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.repository_id)
        .bind(row.routing_mode)
        .bind(row.ongoing_task_id)
        .bind(row.prompt_template)
        .bind(row.task_name)
        .bind(row.tag_ids)
        .bind(row.run_as_user_id)
        .bind(row.agent_type)
        .bind(row.scope)
        .bind(row.team_id)
        .bind(row.owner_user_id)
        .fetch_one(&self.0)
        .await
    }
}

#[async_trait::async_trait]
impl TaskRouteRepository for PgTaskRouteRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskRoute>, sqlx::Error> {
        PgTaskRouteRepository::find_by_id(self, id).await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        repository_id: Option<&str>,
    ) -> Result<Vec<TaskRoute>, sqlx::Error> {
        PgTaskRouteRepository::list(self, ctx, repository_id).await
    }

    async fn insert(&self, row: NewTaskRoute) -> Result<TaskRoute, sqlx::Error> {
        PgTaskRouteRepository::insert(self, row).await
    }

    async fn update(&self, row: UpdateTaskRouteRow) -> Result<TaskRoute, sqlx::Error> {
        PgTaskRouteRepository::update(self, row).await
    }
}

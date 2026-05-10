//! Persistence for `scheduled_jobs`.

use crate::db::DbPool;

use super::super::models::ScheduledJob;

pub struct PgScheduledJobRepository(pub DbPool);

#[async_trait::async_trait]
pub trait ScheduledJobRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<ScheduledJob>, sqlx::Error>;

    async fn list(&self) -> Result<Vec<ScheduledJob>, sqlx::Error>;

    async fn insert(&self, row: NewScheduledJob) -> Result<ScheduledJob, sqlx::Error>;

    async fn update(&self, row: UpdateScheduledJobRow) -> Result<ScheduledJob, sqlx::Error>;
}

pub struct NewScheduledJob {
    pub id: String,
    pub route_id: String,
    pub schedule_expr: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub next_fire_at: chrono::DateTime<chrono::Utc>,
    pub created_by: String,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct UpdateScheduledJobRow {
    pub id: String,
    pub route_id: String,
    pub schedule_expr: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub next_fire_at: chrono::DateTime<chrono::Utc>,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
}

impl PgScheduledJobRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<ScheduledJob>, sqlx::Error> {
        sqlx::query_as::<_, ScheduledJob>("SELECT * FROM scheduled_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn list(&self) -> Result<Vec<ScheduledJob>, sqlx::Error> {
        sqlx::query_as::<_, ScheduledJob>(
            "SELECT * FROM scheduled_jobs ORDER BY next_fire_at ASC, created_at DESC",
        )
        .fetch_all(&self.0)
        .await
    }

    pub async fn insert(&self, row: NewScheduledJob) -> Result<ScheduledJob, sqlx::Error> {
        sqlx::query_as::<_, ScheduledJob>(
            r#"INSERT INTO scheduled_jobs
               (id, route_id, schedule_expr, timezone, message_template,
                run_as_user_id, active, next_fire_at, created_by, scope, team_id,
                owner_user_id, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.route_id)
        .bind(row.schedule_expr)
        .bind(row.timezone)
        .bind(row.message_template)
        .bind(row.run_as_user_id)
        .bind(row.active)
        .bind(row.next_fire_at)
        .bind(row.created_by)
        .bind(row.scope)
        .bind(row.team_id)
        .bind(row.owner_user_id)
        .bind(row.created_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn update(&self, row: UpdateScheduledJobRow) -> Result<ScheduledJob, sqlx::Error> {
        sqlx::query_as::<_, ScheduledJob>(
            r#"UPDATE scheduled_jobs
               SET route_id = $2,
                   schedule_expr = $3,
                   timezone = $4,
                   message_template = $5,
                   run_as_user_id = $6,
                   active = $7,
                   next_fire_at = $8,
                   scope = $9,
                   team_id = $10,
                   owner_user_id = $11,
                   updated_at = now()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.route_id)
        .bind(row.schedule_expr)
        .bind(row.timezone)
        .bind(row.message_template)
        .bind(row.run_as_user_id)
        .bind(row.active)
        .bind(row.next_fire_at)
        .bind(row.scope)
        .bind(row.team_id)
        .bind(row.owner_user_id)
        .fetch_one(&self.0)
        .await
    }
}

#[async_trait::async_trait]
impl ScheduledJobRepository for PgScheduledJobRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<ScheduledJob>, sqlx::Error> {
        PgScheduledJobRepository::find_by_id(self, id).await
    }

    async fn list(&self) -> Result<Vec<ScheduledJob>, sqlx::Error> {
        PgScheduledJobRepository::list(self).await
    }

    async fn insert(&self, row: NewScheduledJob) -> Result<ScheduledJob, sqlx::Error> {
        PgScheduledJobRepository::insert(self, row).await
    }

    async fn update(&self, row: UpdateScheduledJobRow) -> Result<ScheduledJob, sqlx::Error> {
        PgScheduledJobRepository::update(self, row).await
    }
}

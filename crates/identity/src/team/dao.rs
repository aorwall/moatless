//! Team DAO — queries for team and membership persistence.

use async_trait::async_trait;
use uuid::Uuid;

use super::models::{Team, TeamMember};
use moatless_core::db::DbPool;

/// DAO trait for team persistence.
#[cfg_attr(
    test,
    allow(
        clippy::ref_option_ref,
        reason = "mockall predicates borrow lifetime-parameterized optional arguments"
    )
)]
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TeamDao: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<Team>, sqlx::Error>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Team>, sqlx::Error>;
    async fn list_all(&self) -> Result<Vec<Team>, sqlx::Error>;
    async fn insert(&self, team: &Team) -> Result<(), sqlx::Error>;
    async fn update<'a>(
        &self,
        id: &str,
        name: &str,
        description: Option<&'a str>,
    ) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<(), sqlx::Error>;

    async fn find_teams_for_user(&self, user_id: &Uuid) -> Result<Vec<Team>, sqlx::Error>;
    async fn find_team_ids_for_user(&self, user_id: &Uuid) -> Result<Vec<String>, sqlx::Error>;
    async fn find_members(&self, team_id: &str) -> Result<Vec<TeamMember>, sqlx::Error>;
    async fn add_member(
        &self,
        team_id: &str,
        user_id: &Uuid,
        role: &str,
    ) -> Result<(), sqlx::Error>;
    async fn remove_member(&self, team_id: &str, user_id: &Uuid) -> Result<(), sqlx::Error>;
    async fn is_member(&self, team_id: &str, user_id: &Uuid) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`TeamDao`].
pub struct PgTeamDao(pub DbPool);

#[async_trait]
impl TeamDao for PgTeamDao {
    async fn find_by_id(&self, id: &str) -> Result<Option<Team>, sqlx::Error> {
        sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Team>, sqlx::Error> {
        sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE slug = $1")
            .bind(slug)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_all(&self) -> Result<Vec<Team>, sqlx::Error> {
        sqlx::query_as::<_, Team>("SELECT * FROM teams ORDER BY name ASC")
            .fetch_all(&self.0)
            .await
    }

    async fn insert(&self, team: &Team) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO teams (id, name, slug, description, external_id, external_provider, is_default, settings, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(&team.id)
        .bind(&team.name)
        .bind(&team.slug)
        .bind(&team.description)
        .bind(&team.external_id)
        .bind(&team.external_provider)
        .bind(team.is_default)
        .bind(&team.settings)
        .bind(team.created_at)
        .bind(team.updated_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn update<'a>(
        &self,
        id: &str,
        name: &str,
        description: Option<&'a str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE teams SET name = $2, description = $3, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(name)
        .bind(description)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM teams WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn find_teams_for_user(&self, user_id: &Uuid) -> Result<Vec<Team>, sqlx::Error> {
        sqlx::query_as::<_, Team>(
            r#"SELECT t.id, t.name, t.slug, t.description, t.external_id, t.external_provider, t.is_default, t.settings, t.created_at, t.updated_at
               FROM teams t
               JOIN team_members tm ON t.id = tm.team_id
               WHERE tm.user_id = $1
               ORDER BY t.name ASC"#,
        )
        .bind(user_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_team_ids_for_user(&self, user_id: &Uuid) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT team_id FROM team_members WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(&self.0)
                .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn find_members(&self, team_id: &str) -> Result<Vec<TeamMember>, sqlx::Error> {
        sqlx::query_as::<_, TeamMember>(
            "SELECT * FROM team_members WHERE team_id = $1 ORDER BY created_at ASC",
        )
        .bind(team_id)
        .fetch_all(&self.0)
        .await
    }

    async fn add_member(
        &self,
        team_id: &str,
        user_id: &Uuid,
        role: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO team_members (team_id, user_id, role)
               VALUES ($1, $2, $3)
               ON CONFLICT (team_id, user_id) DO UPDATE SET role = $3"#,
        )
        .bind(team_id)
        .bind(user_id)
        .bind(role)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn remove_member(&self, team_id: &str, user_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM team_members WHERE team_id = $1 AND user_id = $2")
            .bind(team_id)
            .bind(user_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn is_member(&self, team_id: &str, user_id: &Uuid) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM team_members WHERE team_id = $1 AND user_id = $2)",
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }
}

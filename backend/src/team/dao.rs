//! Team DAO — queries for team and membership persistence.

use async_trait::async_trait;

use super::models::{Team, TeamMember};
use crate::db::DbPool;

/// Repository trait for team persistence.
#[async_trait]
pub trait TeamRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<Team>, sqlx::Error>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Team>, sqlx::Error>;
    async fn find_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<Team>, sqlx::Error>;
    async fn list_all(&self) -> Result<Vec<Team>, sqlx::Error>;
    async fn insert(&self, team: &Team) -> Result<(), sqlx::Error>;
    async fn update(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<(), sqlx::Error>;

    async fn find_teams_for_user(&self, user_id: &str) -> Result<Vec<Team>, sqlx::Error>;
    async fn find_team_ids_for_user(&self, user_id: &str) -> Result<Vec<String>, sqlx::Error>;
    async fn find_members(&self, team_id: &str) -> Result<Vec<TeamMember>, sqlx::Error>;
    async fn add_member(&self, team_id: &str, user_id: &str, role: &str)
        -> Result<(), sqlx::Error>;
    async fn remove_member(&self, team_id: &str, user_id: &str) -> Result<(), sqlx::Error>;
    async fn is_member(&self, team_id: &str, user_id: &str) -> Result<bool, sqlx::Error>;

    /// Upsert a team by external_id + provider (for Auth0 group sync).
    async fn upsert_by_external(
        &self,
        external_id: &str,
        provider: &str,
        name: &str,
        slug: &str,
    ) -> Result<Team, sqlx::Error>;
}

/// PostgreSQL implementation of [`TeamRepository`].
pub struct PgTeamRepository(pub DbPool);

#[async_trait]
impl TeamRepository for PgTeamRepository {
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

    async fn find_by_external_id(
        &self,
        external_id: &str,
        provider: &str,
    ) -> Result<Option<Team>, sqlx::Error> {
        sqlx::query_as::<_, Team>(
            "SELECT * FROM teams WHERE external_id = $1 AND external_provider = $2",
        )
        .bind(external_id)
        .bind(provider)
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

    async fn update(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
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

    async fn find_teams_for_user(&self, user_id: &str) -> Result<Vec<Team>, sqlx::Error> {
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

    async fn find_team_ids_for_user(&self, user_id: &str) -> Result<Vec<String>, sqlx::Error> {
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
        user_id: &str,
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

    async fn remove_member(&self, team_id: &str, user_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM team_members WHERE team_id = $1 AND user_id = $2")
            .bind(team_id)
            .bind(user_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn is_member(&self, team_id: &str, user_id: &str) -> Result<bool, sqlx::Error> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM team_members WHERE team_id = $1 AND user_id = $2)",
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_one(&self.0)
        .await?;
        Ok(row.0)
    }

    async fn upsert_by_external(
        &self,
        external_id: &str,
        provider: &str,
        name: &str,
        slug: &str,
    ) -> Result<Team, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        sqlx::query(
            r#"INSERT INTO teams (id, name, slug, external_id, external_provider, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (external_id, external_provider)
                   WHERE external_id IS NOT NULL AND external_provider IS NOT NULL
               DO UPDATE SET name = $2, updated_at = $7"#,
        )
        .bind(&id)
        .bind(name)
        .bind(slug)
        .bind(external_id)
        .bind(provider)
        .bind(now)
        .bind(now)
        .execute(&self.0)
        .await?;

        // Fetch the actual row (may be existing or newly created)
        let team = sqlx::query_as::<_, Team>(
            "SELECT * FROM teams WHERE external_id = $1 AND external_provider = $2",
        )
        .bind(external_id)
        .bind(provider)
        .fetch_one(&self.0)
        .await?;

        Ok(team)
    }
}

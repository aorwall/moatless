//! User table queries.

use chrono::NaiveDateTime;
use super::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub struct UserRow {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub role: String,
    pub created_at: NaiveDateTime,
    pub last_login_at: NaiveDateTime,
}

/// Get user by ID.
pub async fn get_user(pool: &DbPool, user_id: &str) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

/// Upsert user (insert or update on conflict).
pub async fn upsert_user(pool: &DbPool, user: &UserRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO users (id, login, email, name, avatar_url, provider, role, created_at, last_login_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (id) DO UPDATE SET
            login = EXCLUDED.login,
            email = EXCLUDED.email,
            name = EXCLUDED.name,
            avatar_url = EXCLUDED.avatar_url,
            last_login_at = EXCLUDED.last_login_at
        "#,
    )
    .bind(&user.id)
    .bind(&user.login)
    .bind(&user.email)
    .bind(&user.name)
    .bind(&user.avatar_url)
    .bind(&user.provider)
    .bind(&user.role)
    .bind(user.created_at)
    .bind(user.last_login_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// List all users.
pub async fn list_users(pool: &DbPool) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users ORDER BY login ASC")
        .fetch_all(pool)
        .await
}

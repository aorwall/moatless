use super::{error::AuthError, users::User};
use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use moatless_types::user::UserRole;
use uuid::Uuid;
#[derive(sqlx::FromRow)]
pub(super) struct AuthTransaction {
    pub(super) nonce: String,
    pub(super) pkce_verifier: String,
    pub(super) redirect_after_login: Option<String>,
    pub(super) cli_callback_port: Option<i32>,
}

pub(super) struct NewTransaction<'a> {
    pub provider_id: &'a str,
    pub state_hash: Vec<u8>,
    pub nonce: &'a str,
    pub pkce_verifier: &'a str,
    pub redirect_after_login: Option<String>,
    pub cli_callback_port: Option<i32>,
    pub expires_at: DateTime<Utc>,
}
pub(super) async fn insert_transaction(
    db: &DbPool,
    input: NewTransaction<'_>,
) -> Result<(), sqlx::Error> {
    let NewTransaction {
        provider_id,
        state_hash,
        nonce,
        pkce_verifier,
        redirect_after_login,
        cli_callback_port,
        expires_at,
    } = input;
    sqlx::query(
        r#"INSERT INTO auth_transactions
               (id, provider_id, state_hash, nonce, pkce_verifier,
                redirect_after_login, cli_callback_port, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(Uuid::new_v4())
    .bind(provider_id)
    .bind(state_hash)
    .bind(nonce)
    .bind(pkce_verifier)
    .bind(redirect_after_login)
    .bind(cli_callback_port)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub(super) async fn consume_transaction(
    db: &DbPool,
    provider_id: &str,
    state_hash: Vec<u8>,
) -> Result<AuthTransaction, AuthError> {
    sqlx::query_as::<_, AuthTransaction>(
        r#"DELETE FROM auth_transactions
               WHERE provider_id = $1
                 AND state_hash = $2
                 AND consumed_at IS NULL
                 AND expires_at > now()
               RETURNING nonce, pkce_verifier, redirect_after_login, cli_callback_port"#,
    )
    .bind(provider_id)
    .bind(state_hash)
    .fetch_optional(db)
    .await?
    .ok_or(AuthError::InvalidState)
}

pub(super) async fn find_identity(
    db: &DbPool,
    issuer: &str,
    subject: &str,
) -> Result<Option<(Uuid, String, bool)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, String, bool)>(
        r#"SELECT users.id, users.status, users.is_bot
               FROM oidc_identities
               JOIN users ON users.id = oidc_identities.user_id
               WHERE oidc_identities.issuer = $1
                 AND oidc_identities.subject = $2"#,
    )
    .bind(issuer)
    .bind(subject)
    .fetch_optional(db)
    .await
}

pub(super) async fn record_login(
    db: &DbPool,
    issuer: &str,
    subject: &str,
    user_id: Uuid,
) -> Result<User, AuthError> {
    sqlx::query(
        "UPDATE oidc_identities SET last_login_at = now() WHERE issuer = $1 AND subject = $2",
    )
    .bind(issuer)
    .bind(subject)
    .execute(db)
    .await?;
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
        .map_err(AuthError::Database)
}

pub(super) async fn link_first_login(
    db: &DbPool,
    provider_id: &str,
    issuer: &str,
    subject: &str,
    email: &str,
    auto_link: bool,
    auto_provision: bool,
) -> Result<User, AuthError> {
    let mut tx = db.begin().await?;
    let existing_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM users WHERE email_normalized = $1 AND NOT is_bot FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut *tx)
    .await?;
    if existing_status
        .as_deref()
        .is_some_and(|status| status != "active")
    {
        return Err(AuthError::Forbidden);
    }
    let existing = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE email_normalized = $1 AND status = 'active' AND NOT is_bot",
    )
    .bind(email)
    .fetch_optional(&mut *tx)
    .await?;

    let user = match existing {
        Some(user) if auto_link => user,
        Some(_) => {
            return Err(AuthError::Conflict("account_link_required".to_string()));
        }
        None if auto_provision => {
            let id = Uuid::new_v4();
            let login = email
                .split_once('@')
                .map_or(email, |(local, _)| local)
                .to_string();
            sqlx::query_as::<_, User>(
                r#"INSERT INTO users
                       (id, login, email, name, avatar_url, provider,
                        provider_user_id, role, created_at, last_login_at, is_bot)
                       VALUES ($1, $2, $3, NULL, NULL, 'oidc', NULL, $4, now(), now(), false)
                       RETURNING *"#,
            )
            .bind(id)
            .bind(login)
            .bind(email)
            .bind(UserRole::User.as_str())
            .fetch_one(&mut *tx)
            .await?
        }
        None => return Err(AuthError::Forbidden),
    };

    sqlx::query(
        r#"INSERT INTO oidc_identities
               (id, user_id, provider_id, issuer, subject, last_login_at)
               VALUES ($1, $2, $3, $4, $5, now())"#,
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(provider_id)
    .bind(issuer)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(user)
}

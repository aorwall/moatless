//! Provider token DAO — queries for credential resolution.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
use moatless_core::db::DbPool;
use moatless_types::agent::CodexScope;

/// What the policy half decided to do with a Codex blob it was handed.
pub enum CodexBlobRefresh {
    /// The stored blob is already good. Write nothing.
    Fresh,
    /// Replace the blob, and the refresh token beside it when one rotated.
    Rotated {
        blob: String,
        refresh_token: Option<String>,
    },
    /// The refresh failed for good. Record `reason` on the row.
    Permanent { reason: String },
    /// The refresh failed for now. Keep the stored blob.
    Transient,
    /// The stored blob could not be read at all.
    Unreadable,
}

/// The policy half of a Codex blob refresh: is this blob stale, and what
/// replaces it. The store owns the row, the advisory lock, the
/// compare-and-swap, and the failure marker; the caller owns the staleness rule
/// and the network call.
#[async_trait]
pub trait CodexBlobRefresher: Send + Sync {
    async fn refresh(&self, blob: &str) -> CodexBlobRefresh;
}

/// What a [`ProviderTokenDao::refresh_codex_blob`] left on the row.
pub enum CodexBlobOutcome {
    /// The stored blob, unchanged — already fresh, a transient failure, or
    /// another replica held the lock and is doing the work.
    Unchanged { blob: String },
    /// The blob was replaced and the new value is persisted.
    Rotated { blob: String },
    /// The refresh failed permanently. `reason` is now recorded on the row;
    /// `was_already_failing` is true when the row already carried the same
    /// reason, so a caller can log the transition rather than every retry.
    PermanentFailure {
        reason: String,
        was_already_failing: bool,
    },
    /// The stored blob could not be read.
    Unreadable { blob: String },
}

/// DAO trait for provider token credential resolution.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` / `Option<&Value>` in async-trait
/// methods.  Tests use `PgProviderTokenDao` via `#[sqlx::test]`.
// reason: provider-token upserts already pass 7+ atomic columns; wrapping
// into a struct would require touching every call-site and impl for no real
// readability gain.
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait ProviderTokenDao: Send + Sync {
    /// Find a provider token (`access_token` only) by `user_id` and provider name.
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error>;

    /// Find the full provider token row by `user_id` and provider name.
    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a provider token (insert or update on conflict). Inserts with
    /// `auth_method = 'manual'` and leaves an existing row's `auth_method`
    /// untouched — use [`upsert_token_with_auth_method`] to set it explicitly.
    ///
    /// [`upsert_token_with_auth_method`]: ProviderTokenDao::upsert_token_with_auth_method
    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Upsert a provider token, setting `auth_method` on both insert and
    /// update. Used for reference rows (`secret_ref`) whose resolution branch
    /// must switch even when a row already exists.
    ///
    /// `refresh_token` and `expires_at` are written as given, `None` included:
    /// they describe the credential this call installs, and a leftover expiry
    /// from the one it replaces dates the wrong token.
    async fn upsert_token_with_auth_method(
        &self,
        user_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a provider token for a user.
    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error>;

    /// The user who connected `provider_user_id` at `provider`.
    async fn find_user_by_provider_account(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    /// The same, addressed by the provider's own handle.
    async fn find_user_by_provider_login(
        &self,
        provider: &str,
        provider_login: &str,
    ) -> Result<Option<String>, sqlx::Error>;

    // ---- the app-level row: one per provider, no user and no team ----

    /// The deployment-wide OAuth row for `provider`.
    async fn find_app_token(&self, provider: &str) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert the deployment-wide OAuth row for `provider`.
    async fn upsert_app_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error>;

    // ---- Scope-aware codex methods ----

    /// Find the codex `provider_tokens` row addressed by `scope`. Returns the
    /// full row (including `metadata`) so callers can build a
    /// the Agent-harness credential status response without a follow-up read.
    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a codex blob at the requested scope. The unified shape replaces
    /// the user / global triplet so the user / admin endpoints share one
    /// persistence path.
    async fn upsert_codex_blob(
        &self,
        scope: &CodexScope,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete the codex row at the requested scope.
    async fn delete_codex_blob(&self, scope: &CodexScope) -> Result<(), sqlx::Error>;

    /// Every codex scope whose row is still healthy — no recorded permanent
    /// refresh failure. The partial index `idx_provider_tokens_codex_healthy`
    /// (migration `0059`) backs the predicate, so this stays O(touched rows).
    async fn healthy_codex_scopes(&self) -> Result<Vec<CodexScope>, sqlx::Error>;

    /// Bring the codex row at `scope` up to date, asking `refresher` what to
    /// write.
    ///
    /// One transaction: take a Postgres advisory lock keyed on the scope, read
    /// the row `FOR UPDATE`, hand the blob to `refresher`, then compare-and-swap
    /// the answer back. A lock another replica already holds, and a
    /// compare-and-swap another writer won, both report the value that writer
    /// left. `None` when no row addresses `scope`.
    async fn refresh_codex_blob(
        &self,
        scope: &CodexScope,
        refresher: &(dyn CodexBlobRefresher + '_),
    ) -> Result<Option<CodexBlobOutcome>, sqlx::Error>;

    // ---- github rows, addressed by the user who owns them ----

    /// Find `user_id`'s github `provider_tokens` row.
    async fn find_github_row(&self, user_id: &str) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert `user_id`'s github row.
    async fn upsert_github_row(
        &self,
        user_id: &str,
        auth_method: &str,
        access_token: &str,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        provider_user_id: Option<&str>,
        provider_login: Option<&str>,
    ) -> Result<(), sqlx::Error>;

    /// Delete `user_id`'s github row.
    async fn delete_github_row(&self, user_id: &str) -> Result<(), sqlx::Error>;

    /// Whether `user_id` holds a github row, returning only addressing info —
    /// no token material, no decrypt, no write. Drives `/me`'s setup
    /// evaluation.
    async fn find_github_scope_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error>;
}

/// PostgreSQL implementation of [`ProviderTokenDao`].
pub struct PgProviderTokenDao(pub DbPool);

#[async_trait]
impl ProviderTokenDao for PgProviderTokenDao {
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderAccessToken>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            "SELECT * FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   user_id, provider, access_token, refresh_token, token_type, metadata,
                   auth_method, connected_at, updated_at, scope, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, 'manual', NOW(), NOW(), 'user', $1)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6,
                             scope = 'user', owner_user_id = $1, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn upsert_token_with_auth_method(
        &self,
        user_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   user_id, provider, access_token, refresh_token, token_type, metadata,
                   auth_method, connected_at, updated_at, scope, owner_user_id, expires_at
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW(), 'user', $1, $8)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6,
                             auth_method = $7, scope = 'user', owner_user_id = $1, updated_at = NOW(),
                             expires_at = $8"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .bind(auth_method)
        .bind(expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM provider_tokens WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(provider)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        let where_clause = codex_where_clause(scope, "$1");
        // `ProviderToken::user_id` is `NOT NULL` in the Rust struct so
        // coalesce NULL to '' for global rows.
        let sql = format!(
            r#"SELECT COALESCE(user_id, '') AS user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE {where_clause}"#,
        );
        sqlx::query_as::<_, ProviderToken>(AssertSqlSafe(sql))
            .bind(codex_bind_value(scope))
            .fetch_optional(&self.0)
            .await
    }

    async fn upsert_codex_blob(
        &self,
        scope: &CodexScope,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        match scope {
            CodexScope::User(user_id) => {
                // User scope: delegate to upsert_token so the user path stays
                // in one place.
                self.upsert_token(
                    user_id,
                    "codex",
                    access_token,
                    refresh_token,
                    token_type,
                    metadata,
                )
                .await
            }
            CodexScope::Global => {
                // Global codex row: user_id=NULL, team_id=NULL, scope='global'.
                // The unique constraint provider_tokens_scope_uq is
                // UNIQUE NULLS NOT DISTINCT (user_id, provider, team_id),
                // so the NULL/NULL combination is unique per provider.
                sqlx::query(
                    r#"INSERT INTO provider_tokens (
                           provider, auth_method, access_token, refresh_token,
                           token_type, metadata, connected_at, updated_at, scope,
                           owner_user_id
                       )
                       VALUES ('codex', 'manual', $1, $2, $3, $4, NOW(), NOW(),
                               'global', NULL)
                       ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
                       DO UPDATE SET access_token = $1, refresh_token = $2,
                                     token_type = $3, metadata = $4,
                                     auth_method = 'manual', scope = 'global',
                                     owner_user_id = NULL, updated_at = NOW()"#,
                )
                .bind(access_token)
                .bind(refresh_token)
                .bind(token_type)
                .bind(metadata)
                .execute(&self.0)
                .await?;
                Ok(())
            }
        }
    }

    async fn delete_codex_blob(&self, scope: &CodexScope) -> Result<(), sqlx::Error> {
        let where_clause = codex_where_clause(scope, "$1");
        let sql = format!("DELETE FROM provider_tokens WHERE {where_clause}");
        sqlx::query(AssertSqlSafe(sql))
            .bind(codex_bind_value(scope))
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn healthy_codex_scopes(&self) -> Result<Vec<CodexScope>, sqlx::Error> {
        let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT user_id, scope FROM provider_tokens \
             WHERE provider = 'codex' \
               AND metadata->>'refresh_failure' IS NULL",
        )
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(user_id, scope)| match (scope.as_deref(), user_id) {
                (Some("user"), Some(user_id)) => Some(CodexScope::user(user_id)),
                (Some("global"), _) => Some(CodexScope::Global),
                _ => None,
            })
            .collect())
    }

    async fn find_user_by_provider_account(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT user_id FROM provider_tokens \
             WHERE provider = $1 AND provider_user_id = $2 \
             LIMIT 1",
        )
        .bind(provider)
        .bind(provider_user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_user_by_provider_login(
        &self,
        provider: &str,
        provider_login: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT user_id FROM provider_tokens \
             WHERE provider = $1 AND provider_login = $2 \
             LIMIT 1",
        )
        .bind(provider)
        .bind(provider_login)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_app_token(&self, provider: &str) -> Result<Option<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            r#"SELECT COALESCE(user_id, '') AS user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE provider = $1 AND user_id IS NULL AND team_id IS NULL
               LIMIT 1"#,
        )
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_app_token(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   provider, auth_method, access_token, refresh_token, token_type,
                   metadata, connected_at, updated_at, scope, owner_user_id, expires_at
               )
               VALUES ($1, 'oauth', $2, $3, $4, $5, NOW(), NOW(), 'global', NULL, $6)
               ON CONFLICT (user_id, provider, team_id) DO UPDATE SET
                   access_token = $2, refresh_token = $3, token_type = $4,
                   metadata = $5, auth_method = 'oauth', scope = 'global',
                   owner_user_id = NULL, updated_at = NOW(), expires_at = $6"#,
        )
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .bind(expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn refresh_codex_blob(
        &self,
        scope: &CodexScope,
        refresher: &(dyn CodexBlobRefresher + '_),
    ) -> Result<Option<CodexBlobOutcome>, sqlx::Error> {
        let db = &self.0;
        let mut tx = db.begin().await?;

        // `pg_try_advisory_xact_lock` releases when the transaction ends. When
        // another replica already holds it, drop the transaction and read
        // whatever they end up writing rather than piling up waiters on a slow
        // refresh. The key embeds the scope kind, so a user named "foo" and a
        // team named "foo" do not collide.
        let acquired: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(codex_lock_key(scope))
                .fetch_one(&mut *tx)
                .await?;

        let scope_where = codex_where_clause(scope, "$1");
        let scope_bind = codex_bind_value(scope);

        if !acquired {
            tx.rollback().await?;
            let read_sql = format!("SELECT access_token FROM provider_tokens WHERE {scope_where}");
            let row: Option<(String,)> = sqlx::query_as(AssertSqlSafe(read_sql))
                .bind(scope_bind)
                .fetch_optional(db)
                .await?;
            return Ok(row.map(|(blob,)| CodexBlobOutcome::Unchanged { blob }));
        }

        // Lock the row for the duration of the refresh. Read the recorded
        // failure reason alongside it so the permanent arm can report whether
        // this is a transition into the terminal state or a repeat.
        let lock_sql = format!(
            "SELECT access_token, metadata->'refresh_failure'->>'reason' AS prev_fail_reason \
             FROM provider_tokens WHERE {scope_where} FOR UPDATE",
        );
        let row: Option<(String, Option<String>)> = sqlx::query_as(AssertSqlSafe(lock_sql))
            .bind(scope_bind)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((stored_blob, prev_fail_reason)) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        match refresher.refresh(&stored_blob).await {
            CodexBlobRefresh::Fresh | CodexBlobRefresh::Transient => {
                tx.rollback().await?;
                Ok(Some(CodexBlobOutcome::Unchanged { blob: stored_blob }))
            }
            CodexBlobRefresh::Unreadable => {
                tx.rollback().await?;
                Ok(Some(CodexBlobOutcome::Unreadable { blob: stored_blob }))
            }
            CodexBlobRefresh::Rotated {
                blob,
                refresh_token,
            } => {
                // Compare-and-swap: succeed only while the row still holds what
                // we read. `$1..$3` are the SET params and the CAS check; the
                // scope predicate binds `$4`.
                let cas_where = codex_where_clause(scope, "$4");
                let cas_sql = format!(
                    "UPDATE provider_tokens \
                     SET access_token = $1, \
                         refresh_token = COALESCE($2, refresh_token), \
                         metadata = COALESCE(metadata, '{{}}'::jsonb) - 'refresh_failure', \
                         updated_at = NOW() \
                     WHERE {cas_where} AND access_token = $3",
                );
                let updated = sqlx::query(AssertSqlSafe(cas_sql))
                    .bind(&blob)
                    .bind(refresh_token.as_deref())
                    .bind(&stored_blob)
                    .bind(scope_bind)
                    .execute(&mut *tx)
                    .await?;

                if updated.rows_affected() == 0 {
                    // Another writer got there first; commit the empty
                    // transaction and report their value.
                    tx.commit().await?;
                    let read_sql =
                        format!("SELECT access_token FROM provider_tokens WHERE {scope_where}");
                    let row: Option<(String,)> = sqlx::query_as(AssertSqlSafe(read_sql))
                        .bind(scope_bind)
                        .fetch_optional(db)
                        .await?;
                    return Ok(row.map(|(blob,)| CodexBlobOutcome::Unchanged { blob }));
                }

                tx.commit().await?;
                Ok(Some(CodexBlobOutcome::Rotated { blob }))
            }
            CodexBlobRefresh::Permanent { reason } => {
                let failure = serde_json::json!({
                    "reason": reason,
                    "at": chrono::Utc::now().to_rfc3339(),
                });
                // SET binds `$1`; the scope predicate shifts to `$2`.
                let fail_where = codex_where_clause(scope, "$2");
                let fail_sql = format!(
                    "UPDATE provider_tokens \
                     SET metadata = jsonb_set(COALESCE(metadata, '{{}}'::jsonb), '{{refresh_failure}}', $1, true), \
                         updated_at = NOW() \
                     WHERE {fail_where}",
                );
                sqlx::query(AssertSqlSafe(fail_sql))
                    .bind(&failure)
                    .bind(scope_bind)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                let was_already_failing = prev_fail_reason.as_deref() == Some(reason.as_str());
                Ok(Some(CodexBlobOutcome::PermanentFailure {
                    reason,
                    was_already_failing,
                }))
            }
        }
    }

    async fn find_github_row(&self, user_id: &str) -> Result<Option<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            r#"SELECT user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE user_id = $1 AND provider = 'github' AND scope = 'user'"#,
        )
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_github_row(
        &self,
        user_id: &str,
        auth_method: &str,
        access_token: &str,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        provider_user_id: Option<&str>,
        provider_login: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.upsert_token_with_auth_method(
            user_id,
            "github",
            auth_method,
            access_token,
            None,
            token_type,
            metadata,
            None,
        )
        .await?;
        if provider_user_id.is_some() || provider_login.is_some() {
            sqlx::query(
                r#"UPDATE provider_tokens
                   SET provider_user_id = $2, provider_login = $3
                   WHERE user_id = $1 AND provider = 'github' AND scope = 'user'"#,
            )
            .bind(user_id)
            .bind(provider_user_id)
            .bind(provider_login)
            .execute(&self.0)
            .await?;
        }
        Ok(())
    }

    async fn delete_github_row(&self, user_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM provider_tokens \
             WHERE user_id = $1 AND provider = 'github' AND scope = 'user'",
        )
        .bind(user_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_github_scope_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
        sqlx::query_as::<_, GithubTokenPresence>(
            r#"SELECT auth_method,
                      metadata->>'login' AS login,
                      (metadata->>'refresh_failure') IS NOT NULL AS refresh_failure
               FROM provider_tokens
               WHERE provider = 'github' AND scope = 'user' AND user_id = $1"#,
        )
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }
}

/// Stable key shared by the advisory lock and in-process credential mutex.
pub fn codex_lock_key(scope: &CodexScope) -> String {
    match scope {
        CodexScope::User(id) => format!("codex-refresh:user:{id}"),
        CodexScope::Global => "codex-refresh:global".to_string(),
    }
}

/// Bind value for the `WHERE` fragment's single `$N` placeholder.
/// User scope carries the `user_id`; Global uses an empty string sentinel
/// (the clause itself uses IS NULL checks, not a bound value).
fn codex_bind_value(scope: &CodexScope) -> &str {
    match scope {
        CodexScope::User(id) => id.as_str(),
        CodexScope::Global => "",
    }
}

/// Build the `WHERE` clause fragment that addresses this scope's row in
/// `provider_tokens`, using `placeholder` for the dynamic identifier.
/// Callers paste the result into the larger SQL string and bind
/// [`codex_bind_value`] at the same positional parameter index.
///
/// Note: Global scope uses `IS NULL` checks so the bind value is unused
/// for that variant; callers should still bind it (empty string) to keep
/// positional parameter counts consistent.
fn codex_where_clause(scope: &CodexScope, placeholder: &str) -> String {
    match scope {
        CodexScope::User(_) => {
            format!("user_id = {placeholder} AND provider = 'codex' AND scope = 'user'")
        }
        CodexScope::Global => {
            "user_id IS NULL AND team_id IS NULL AND provider = 'codex' AND scope = 'global'"
                .to_string()
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn lock_key_is_distinct_across_scopes_with_same_id() {
        let user = codex_lock_key(&CodexScope::user("foo"));
        let global = codex_lock_key(&CodexScope::Global);
        assert_eq!(user, "codex-refresh:user:foo");
        assert_eq!(global, "codex-refresh:global");
        assert_ne!(user, global);
        assert!(user.contains("user"));
        assert!(global.contains("global"));
    }

    #[test]
    fn where_clause_per_scope_is_well_formed() {
        let u = codex_where_clause(&CodexScope::user("u1"), "$1");
        assert!(u.contains("user_id = $1"));
        assert!(u.contains("scope = 'user'"));

        let g = codex_where_clause(&CodexScope::Global, "$1");
        assert!(g.contains("user_id IS NULL"));
        assert!(g.contains("team_id IS NULL"));
        assert!(g.contains("scope = 'global'"));
    }

    #[test]
    fn bind_value_is_the_scope_identifier() {
        assert_eq!(codex_bind_value(&CodexScope::user("alice")), "alice");
        assert_eq!(codex_bind_value(&CodexScope::Global), "");
    }
}

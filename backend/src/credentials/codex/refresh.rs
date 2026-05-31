//! Codex-specific refresh path for [`CredentialService`].
//!
//! This file adds a second `impl CredentialService` block holding every
//! codex-aware method. The struct itself is still declared in
//! `credentials/mod.rs` — Rust allows multiple `impl` blocks for the same type
//! across modules within a crate, so callers see the same public API surface
//! (`state.credentials_svc.ensure_fresh_codex(...)`) regardless of which module
//! the method lives in.

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::credentials::codex::oauth;
use crate::credentials::codex::scope::CodexScope;
use crate::credentials::{CredentialError, CredentialService};
use crate::db::DbPool;

impl CredentialService {
    /// Just-in-time Codex auth.json refresh, addressed by `scope`.
    ///
    /// Behaviour (uniform across user / team / global scopes):
    /// 1. If another *open* codex task within this scope (other than
    ///    `except_task_id`) is in flight, return the stored blob unchanged
    ///    — the sandbox watcher will rotate it. Avoids racing the CLI for
    ///    the single-use refresh_token.
    /// 2. Acquire per-scope mutex + Postgres advisory transaction lock, then
    ///    `SELECT … FOR UPDATE` the row addressed by the scope's WHERE
    ///    clause.
    /// 3. Check `should_refresh` (access-token exp <= now + 5min, or
    ///    last_refresh < now - 8d). If not, return the stored blob.
    /// 4. POST `refresh_token` to `auth.openai.com`. On success, merge the
    ///    rotated fields into the blob, CAS-update the row, broadcast the
    ///    new blob to every running sandbox whose effective user resolves
    ///    through this scope.
    /// 5. On permanent failure, write `metadata.refresh_failure` and return
    ///    `CredentialError::NotFound`. On transient failure, return the
    ///    stored (stale) blob.
    pub async fn ensure_fresh_codex(
        &self,
        scope: CodexScope,
        except_task_id: Option<&str>,
    ) -> Result<String, CredentialError> {
        let lock = self
            .acquire_lock(&scope.lock_key(), "CODEX_AUTH_JSON")
            .await;
        let _guard = lock.lock().await;

        // Step 1: skip the backend refresh when the sandbox watcher is the
        // active refresher for any task within this scope.
        if self
            .any_other_open_codex_task(&scope, except_task_id)
            .await
        {
            debug!(
                scope = %scope.display(),
                "ensure_fresh_codex: skipping backend refresh (open codex task in flight)"
            );
            return self.read_raw_codex(&scope).await;
        }

        // Step 2: require DB + HTTP. Without them we fall back to a raw read.
        let (Some(db), Some(http)) = (self.db.as_ref(), self.http.as_ref()) else {
            return self.read_raw_codex(&scope).await;
        };

        let outcome = codex_refresh_inner(db, http, &scope).await?;
        if let CodexRefreshOutcome::Rotated { ref new_blob } = outcome {
            // Broadcast outside the SQL transaction — fan-out spawns gRPC
            // tasks per running sandbox.
            self.broadcast_to_scope(&scope, None, "CODEX_AUTH_JSON", new_blob)
                .await;
        }
        Ok(outcome.into_blob())
    }

    /// Returns true when any *other* codex-typed task within this scope is
    /// currently `Open`. Best-effort: any DB error is logged and treated as
    /// "no other tasks" so a transient failure can't block every refresh.
    async fn any_other_open_codex_task(
        &self,
        scope: &CodexScope,
        except_task_id: Option<&str>,
    ) -> bool {
        let Some(ref ctx) = self.broadcast_ctx else {
            return false;
        };
        let scope_kind = scope.scope_kind().to_string();
        let scope_id = match scope {
            CodexScope::User(id) | CodexScope::Team(id) => id.clone(),
            CodexScope::Global => String::new(),
        };
        let except = except_task_id.unwrap_or("").to_string();
        match ctx
            .tasks
            .list_open_codex_tasks_for_scope(scope_kind, scope_id, except)
            .await
        {
            Ok(tasks) => !tasks.is_empty(),
            Err(e) => {
                warn!(
                    error = %e,
                    scope = %scope.display(),
                    "ensure_fresh_codex: failed to list codex tasks, skipping in-flight guard"
                );
                false
            }
        }
    }

    /// Resolve `CODEX_AUTH_JSON` with `user > team > global` precedence in
    /// one call.
    ///
    /// Single entry point for callers that don't want to think about scopes:
    /// the resolver in `sandbox::secrets`, the gRPC `GetCredentials` handler,
    /// and any future caller all see one method. The walk lives here so the
    /// "most-specific wins" rule is enforced centrally — callers can't
    /// accidentally invert the precedence by reordering their calls.
    ///
    /// Each scope's blob is refreshed in place via
    /// [`Self::ensure_fresh_codex`]. The first scope that yields a usable
    /// blob wins; scopes with no row or a permanent refresh failure
    /// (`CredentialError::NotFound`) silently fall through. Transient
    /// errors are logged and the walk continues so a single scope's
    /// upstream outage can't block the others.
    pub async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        team_id: Option<&str>,
        except_task_id: Option<&str>,
    ) -> Result<String, CredentialError> {
        let mut scopes: Vec<CodexScope> = Vec::with_capacity(3);
        scopes.push(CodexScope::user(user_id));
        if let Some(team_id) = team_id {
            scopes.push(CodexScope::team(team_id));
        }
        scopes.push(CodexScope::Global);

        let mut last_err: Option<CredentialError> = None;
        for scope in scopes {
            match self.ensure_fresh_codex(scope.clone(), except_task_id).await {
                Ok(blob) => return Ok(blob),
                Err(CredentialError::NotFound(_)) => continue,
                Err(e) => {
                    warn!(
                        error = %e,
                        scope = %scope.display(),
                        "ensure_fresh_codex_for: transient failure, trying next scope"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            CredentialError::NotFound(format!(
                "No codex provider_token found at any scope (user={user_id}, team={team_id:?})"
            ))
        }))
    }

    /// Read the codex blob for `scope` straight from `provider_tokens` with
    /// no refresh. Used as the fallback path when the DB / HTTP plumbing for
    /// refresh is absent (tests) or another replica is the active refresher.
    async fn read_raw_codex(&self, scope: &CodexScope) -> Result<String, CredentialError> {
        let row = self
            .provider_tokens
            .find_codex_blob(scope)
            .await?
            .ok_or_else(|| {
                CredentialError::NotFound(format!("No codex provider_token for {scope}"))
            })?;
        Ok(row.access_token)
    }

    /// Public-facing entry point for HTTP handlers that just saved a
    /// team / global codex blob via the DAO and want to fan the rotation
    /// out to live sandboxes. For user-scope writes, call
    /// [`CredentialService::store`] instead so the standard dedup +
    /// persistence path runs.
    pub async fn broadcast_codex_rotation(&self, scope: &CodexScope, value: &str) {
        self.broadcast_to_scope(scope, None, "CODEX_AUTH_JSON", value)
            .await;
    }
}

// =============================================================================
// Codex refresh — raw SQL implementation
// =============================================================================

/// Outcome of `codex_refresh_inner`. The caller broadcasts only on `Rotated`.
pub(super) enum CodexRefreshOutcome {
    /// The stored blob was already fresh — no HTTP call was made.
    Unchanged { blob: String },
    /// We refreshed the blob and the new value is now persisted.
    Rotated { new_blob: String },
}

impl CodexRefreshOutcome {
    fn into_blob(self) -> String {
        match self {
            Self::Unchanged { blob } | Self::Rotated { new_blob: blob } => blob,
        }
    }
}

/// Soft staleness threshold — refresh when the access token is within this
/// window of expiry.
const CODEX_REFRESH_LEEWAY_SECS: i64 = 5 * 60;
/// Hard staleness threshold — proactively refresh once the blob's recorded
/// `last_refresh` is this far in the past, regardless of access token TTL.
/// Matches upstream Codex CLI's 8-day fallback.
const CODEX_REFRESH_MAX_AGE_SECS: i64 = 8 * 24 * 60 * 60;

/// Decide whether the stored Codex blob needs a refresh.
///
/// Three signals — refresh if any of:
/// - `tokens.access_token` JWT `exp` is within [`CODEX_REFRESH_LEEWAY_SECS`].
/// - `last_refresh` is older than [`CODEX_REFRESH_MAX_AGE_SECS`].
/// - We can't decode the access token at all (treat as expired).
fn codex_should_refresh(blob: &serde_json::Value) -> bool {
    let access_token = blob
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(serde_json::Value::as_str);

    let now = Utc::now();
    if let Some(at) = access_token {
        match oauth::parse_jwt_exp(at) {
            Ok(Some(exp)) => {
                let leeway = chrono::Duration::seconds(CODEX_REFRESH_LEEWAY_SECS);
                if exp <= now + leeway {
                    return true;
                }
            }
            // Decode failure: be conservative and refresh.
            Err(_) => return true,
            // No exp claim: fall through to the last_refresh check.
            Ok(None) => {}
        }
    } else {
        // No access token at all → refresh (might be a bot row we just seeded
        // with refresh-only fields).
        return true;
    }

    if let Some(last_refresh) = blob
        .get("last_refresh")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    {
        let age = now.signed_duration_since(last_refresh.with_timezone(&Utc));
        if age.num_seconds() > CODEX_REFRESH_MAX_AGE_SECS {
            return true;
        }
    }

    false
}

/// Merge rotated tokens into the existing Codex auth.json blob, only
/// overwriting fields the server actually rotated.
fn merge_refreshed_blob(
    existing: &serde_json::Value,
    new: &oauth::RefreshedTokens,
) -> serde_json::Value {
    let mut blob = existing.clone();
    let tokens_map = blob
        .get_mut("tokens")
        .and_then(serde_json::Value::as_object_mut);
    if let Some(tokens) = tokens_map {
        if let Some(ref id_token) = new.id_token {
            tokens.insert(
                "id_token".to_string(),
                serde_json::Value::String(id_token.clone()),
            );
        }
        if let Some(ref access_token) = new.access_token {
            tokens.insert(
                "access_token".to_string(),
                serde_json::Value::String(access_token.clone()),
            );
        }
        if let Some(ref refresh_token) = new.refresh_token {
            tokens.insert(
                "refresh_token".to_string(),
                serde_json::Value::String(refresh_token.clone()),
            );
        }
    }
    if let Some(obj) = blob.as_object_mut() {
        obj.insert(
            "last_refresh".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );
    }
    blob
}

/// Core Codex refresh — single transaction, advisory lock + CAS.
///
/// All raw SQL lives here so [`CredentialService::ensure_fresh_codex`] keeps
/// its high-level shape. Lives outside the `impl` so it can borrow `db` /
/// `http` independently of `&self` (avoids long-lived borrows on the service
/// across the network call).
///
/// Scope-parameterized: the `WHERE` fragment, the advisory-lock seed, and
/// the bind value for the dynamic identifier all flow from `scope`. Every
/// query keeps its positional layout (`$1` is always the scope identifier)
/// so the rest of the binds are constant.
pub(super) async fn codex_refresh_inner(
    db: &DbPool,
    http: &reqwest::Client,
    scope: &CodexScope,
) -> Result<CodexRefreshOutcome, CredentialError> {
    let mut tx = db.begin().await?;

    // Postgres advisory lock keyed on the scope. `pg_try_advisory_xact_lock`
    // is released automatically when the transaction ends. If another
    // replica already holds it, fall back to a raw read so we don't pile
    // up waiters on a slow refresh. The lock key embeds the scope kind so
    // a user named "foo" and a team named "foo" don't collide.
    let lock_key: String = scope.advisory_id_input();
    let acquired: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))",
    )
    .bind(&lock_key)
    .fetch_one(&mut *tx)
    .await?;

    let scope_where = scope.where_clause("$1");
    let scope_bind = scope.bind_value();

    if !acquired {
        // Another replica holds the lock — release the transaction and read
        // whatever they end up writing.
        tx.rollback().await?;
        debug!(
            scope = %scope.display(),
            "ensure_fresh_codex: another replica holds the advisory lock, returning current row"
        );
        let read_sql = format!(
            "SELECT access_token FROM provider_tokens WHERE {scope_where}",
        );
        let row: Option<(String,)> = sqlx::query_as(&read_sql)
            .bind(scope_bind)
            .fetch_optional(db)
            .await?;
        return match row {
            Some((blob,)) => Ok(CodexRefreshOutcome::Unchanged { blob }),
            None => Err(CredentialError::NotFound(format!(
                "No codex provider_token for {scope}"
            ))),
        };
    }

    // Lock the row for the duration of the refresh.
    let lock_sql = format!(
        "SELECT access_token FROM provider_tokens WHERE {scope_where} FOR UPDATE",
    );
    let row: Option<(String,)> = sqlx::query_as(&lock_sql)
        .bind(scope_bind)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((existing_blob_str,)) = row else {
        tx.rollback().await?;
        return Err(CredentialError::NotFound(format!(
            "No codex provider_token for {scope}"
        )));
    };

    let existing_blob: serde_json::Value = match serde_json::from_str(&existing_blob_str) {
        Ok(v) => v,
        Err(e) => {
            tx.rollback().await?;
            return Err(CredentialError::Internal(format!(
                "Stored Codex auth.json for {scope} is not valid JSON: {e}"
            )));
        }
    };

    if !codex_should_refresh(&existing_blob) {
        tx.rollback().await?;
        debug!(
            scope = %scope.display(),
            "ensure_fresh_codex: token still fresh, skipping refresh"
        );
        return Ok(CodexRefreshOutcome::Unchanged {
            blob: existing_blob_str,
        });
    }

    let refresh_token = existing_blob
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CredentialError::Internal(format!(
                "Codex blob for {scope} has no tokens.refresh_token"
            ))
        })?;

    info!(scope = %scope.display(), "ensure_fresh_codex: refreshing Codex tokens");
    let refresh_result = oauth::refresh_chatgpt_tokens(http, refresh_token).await;

    match refresh_result {
        Ok(new_tokens) => {
            let new_blob = merge_refreshed_blob(&existing_blob, &new_tokens);
            let new_blob_str = new_blob.to_string();
            let new_refresh_inner = new_tokens.refresh_token.clone();

            // CAS — only succeed when the row's blob still matches what we
            // read. If the sandbox watcher beat us, retry by re-reading.
            // `$1` is the scope identifier (consumed by `scope_where`); the
            // SET params start at `$2` and the CAS check is `$5`.
            let cas_where = scope.where_clause("$4");
            let cas_sql = format!(
                "UPDATE provider_tokens \
                 SET access_token = $1, \
                     refresh_token = COALESCE($2, refresh_token), \
                     metadata = COALESCE(metadata, '{{}}'::jsonb) - 'refresh_failure', \
                     updated_at = NOW() \
                 WHERE {cas_where} AND access_token = $3",
            );
            let updated = sqlx::query(&cas_sql)
                .bind(&new_blob_str)
                .bind(new_refresh_inner.as_deref())
                .bind(&existing_blob_str)
                .bind(scope_bind)
                .execute(&mut *tx)
                .await?;

            if updated.rows_affected() == 0 {
                // Someone (sandbox watcher) wrote underneath us; commit the
                // empty tx and return their value.
                tx.commit().await?;
                let read_sql = format!(
                    "SELECT access_token FROM provider_tokens WHERE {scope_where}",
                );
                let row: Option<(String,)> = sqlx::query_as(&read_sql)
                    .bind(scope_bind)
                    .fetch_optional(db)
                    .await?;
                debug!(
                    scope = %scope.display(),
                    "ensure_fresh_codex: CAS lost, returning watcher's blob"
                );
                return match row {
                    Some((blob,)) => Ok(CodexRefreshOutcome::Unchanged { blob }),
                    None => Err(CredentialError::NotFound(format!(
                        "No codex provider_token for {scope}"
                    ))),
                };
            }

            tx.commit().await?;
            info!(
                scope = %scope.display(),
                "ensure_fresh_codex: refresh complete, row updated"
            );
            Ok(CodexRefreshOutcome::Rotated {
                new_blob: new_blob_str,
            })
        }
        Err(oauth::RefreshTokenError::Permanent { reason, .. }) => {
            let failure = serde_json::json!({
                "reason": format!("{reason:?}"),
                "at": Utc::now().to_rfc3339(),
            });
            // SET takes `$1` (the failure payload); the scope predicate's
            // bind shifts to `$2`.
            let fail_where = scope.where_clause("$2");
            let fail_sql = format!(
                "UPDATE provider_tokens \
                 SET metadata = jsonb_set(COALESCE(metadata, '{{}}'::jsonb), '{{refresh_failure}}', $1, true), \
                     updated_at = NOW() \
                 WHERE {fail_where}",
            );
            sqlx::query(&fail_sql)
                .bind(&failure)
                .bind(scope_bind)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            warn!(
                scope = %scope.display(),
                ?reason,
                "ensure_fresh_codex: refresh failed permanently — user must reconnect"
            );
            Err(CredentialError::NotFound(format!(
                "Codex token refresh failed permanently for {scope} ({reason:?})"
            )))
        }
        Err(e) => {
            tx.rollback().await?;
            warn!(
                scope = %scope.display(),
                error = %e,
                "ensure_fresh_codex: transient refresh failure, returning stale blob"
            );
            Ok(CodexRefreshOutcome::Unchanged {
                blob: existing_blob_str,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt_with_exp(exp_secs: i64) -> String {
        use base64::Engine as _;
        let header =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({ "exp": exp_secs })).unwrap());
        format!("{header}.{body}.sig")
    }

    #[test]
    fn codex_should_refresh_when_access_token_near_exp() {
        // exp 2 minutes from now → inside the 5-minute leeway.
        let exp = Utc::now().timestamp() + 120;
        let blob = serde_json::json!({
            "tokens": { "access_token": make_jwt_with_exp(exp), "refresh_token": "r" },
            "last_refresh": Utc::now().to_rfc3339(),
        });
        assert!(codex_should_refresh(&blob));
    }

    #[test]
    fn codex_should_not_refresh_when_token_fresh() {
        let exp = Utc::now().timestamp() + 3600; // 1h in future
        let blob = serde_json::json!({
            "tokens": { "access_token": make_jwt_with_exp(exp), "refresh_token": "r" },
            "last_refresh": Utc::now().to_rfc3339(),
        });
        assert!(!codex_should_refresh(&blob));
    }

    #[test]
    fn codex_should_refresh_when_last_refresh_old() {
        let exp = Utc::now().timestamp() + 3600; // access token still fresh
        let stale = Utc::now() - chrono::Duration::days(9);
        let blob = serde_json::json!({
            "tokens": { "access_token": make_jwt_with_exp(exp), "refresh_token": "r" },
            "last_refresh": stale.to_rfc3339(),
        });
        assert!(
            codex_should_refresh(&blob),
            "8-day proactive refresh should fire even if access token is fresh"
        );
    }

    #[test]
    fn codex_should_refresh_when_access_token_undecodable() {
        let blob = serde_json::json!({
            "tokens": { "access_token": "not-a-jwt", "refresh_token": "r" },
        });
        assert!(codex_should_refresh(&blob));
    }

    #[test]
    fn codex_should_refresh_when_access_token_missing() {
        let blob = serde_json::json!({
            "tokens": { "refresh_token": "r" },
        });
        assert!(codex_should_refresh(&blob));
    }

    #[test]
    fn merge_refreshed_blob_only_overwrites_present_fields() {
        let existing = serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": "sk-existing",
            "tokens": {
                "id_token": "old-id",
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "account_id": "acc",
            },
            "last_refresh": "1970-01-01T00:00:00+00:00",
        });
        let new = oauth::RefreshedTokens {
            id_token: None, // upstream didn't rotate id_token
            access_token: Some("new-access".into()),
            refresh_token: Some("new-refresh".into()),
        };

        let merged = merge_refreshed_blob(&existing, &new);

        assert_eq!(merged["OPENAI_API_KEY"], "sk-existing");
        assert_eq!(merged["tokens"]["id_token"], "old-id"); // preserved
        assert_eq!(merged["tokens"]["access_token"], "new-access");
        assert_eq!(merged["tokens"]["refresh_token"], "new-refresh");
        assert_eq!(merged["tokens"]["account_id"], "acc"); // preserved
        assert_ne!(merged["last_refresh"], "1970-01-01T00:00:00+00:00");
    }
}

//! The staleness rule and the OpenAI refresh call, behind `CodexBlobRefresher`.
//!
//! The in-flight guard, the scope-precedence walk and the sandbox broadcast
//! belong to the tasks-band `CredentialSync`, so this module names neither
//! `TaskDao` nor `SandboxConnector`. `ProviderTokenDao::refresh_codex_blob`
//! owns the row, the lock and the compare-and-swap.

use async_trait::async_trait;
use chrono::Utc;
use tracing::{debug, info, warn};

use moatless_types::agent::CodexScope;

use crate::auth::codex_blob::{CodexBlobOutcome, CodexBlobRefresh, CodexBlobRefresher};
use crate::credentials::codex::oauth;
use crate::credentials::{CredentialError, CredentialService};

impl CredentialService {
    /// The pure Codex `auth.json` refresh, serialized on the per-scope mutex.
    ///
    /// No in-flight guard and no broadcast: the tasks-band `CredentialSync`
    /// calls this and then, on [`CodexRefreshOutcome::Rotated`], fans the new
    /// blob out.
    ///
    /// Behaviour:
    /// 1. Check `should_refresh` (access-token exp <= now + 5min, or
    ///    `last_refresh` < now - 8d). If not, return the stored blob unchanged.
    /// 2. POST `refresh_token` to `auth.openai.com`. On success merge the
    ///    rotated fields into the blob.
    /// 3. On permanent failure the store writes `metadata.refresh_failure` and
    ///    this returns `CredentialError::NotFound`. On transient failure, return
    ///    the stored (stale) blob. Without HTTP wired, fall back to a raw read.
    pub async fn refresh_codex_locked(
        &self,
        scope: &CodexScope,
    ) -> Result<CodexRefreshOutcome, CredentialError> {
        let lock = self
            .acquire_lock(
                &crate::auth::codex_blob::codex_lock_key(scope),
                "CODEX_AUTH_JSON",
            )
            .await;
        let _guard = lock.lock().await;

        // Require HTTP. Without it we fall back to a raw read.
        let Some(http) = self.http.as_ref() else {
            let blob = self.read_raw_codex(scope).await?;
            return Ok(CodexRefreshOutcome::Unchanged { blob });
        };

        let refresher = CodexRefresher { http, scope };
        let outcome = self
            .provider_tokens
            .refresh_codex_blob(scope, &refresher)
            .await?
            .ok_or_else(|| {
                CredentialError::NotFound(format!("No codex provider_token for {scope}"))
            })?;

        match outcome {
            CodexBlobOutcome::Unchanged { blob } => Ok(CodexRefreshOutcome::Unchanged { blob }),
            CodexBlobOutcome::Rotated { blob } => {
                info!(
                    scope = %scope.display(),
                    "ensure_fresh_codex: refresh complete, row updated"
                );
                Ok(CodexRefreshOutcome::Rotated { new_blob: blob })
            }
            CodexBlobOutcome::Unreadable { blob } => {
                // The store reports `Unreadable` only for a blob this same
                // parse rejected, so the `Ok` arm does not occur in practice.
                Err(match serde_json::from_str::<serde_json::Value>(&blob) {
                    Err(source) => CredentialError::CodexBlobUnreadable {
                        scope: scope.to_string(),
                        source,
                    },
                    Ok(_) => CredentialError::Internal(format!(
                        "Codex blob for {scope} reported unreadable but parses"
                    )),
                })
            }
            CodexBlobOutcome::PermanentFailure {
                reason,
                was_already_failing,
            } => {
                // WARN only on the transition into (or a change of) the terminal
                // state — a row already marked with the same reason re-fails on
                // every `GetCredentials`, so repeat the notice at DEBUG to avoid
                // flooding the logs. A successful rotation clears the marker, so
                // a genuine new failure WARNs again.
                if was_already_failing {
                    debug!(
                        scope = %scope.display(),
                        reason,
                        "ensure_fresh_codex: refresh still failing permanently (repeat WARN suppressed)"
                    );
                } else {
                    warn!(
                        scope = %scope.display(),
                        reason,
                        "ensure_fresh_codex: refresh failed permanently — user must reconnect"
                    );
                }
                Err(CredentialError::NotFound(format!(
                    "Codex token refresh failed permanently for {scope} ({reason})"
                )))
            }
        }
    }

    /// Read the codex blob for `scope` straight from `provider_tokens` with
    /// no refresh. Used by `CredentialSync` on the in-flight-guard path, and
    /// here as the fallback when the HTTP plumbing for refresh is absent
    /// (tests) or another replica is the active refresher.
    pub async fn read_raw_codex(&self, scope: &CodexScope) -> Result<String, CredentialError> {
        let row = self
            .provider_tokens
            .find_codex_blob(scope)
            .await?
            .ok_or_else(|| {
                CredentialError::NotFound(format!("No codex provider_token for {scope}"))
            })?;
        Ok(row.access_token)
    }
}

/// Outcome of the pure refresh. `CredentialSync` broadcasts only on `Rotated`.
pub enum CodexRefreshOutcome {
    /// The stored blob was already fresh — no HTTP call was made.
    Unchanged { blob: String },
    /// We refreshed the blob and the new value is now persisted.
    Rotated { new_blob: String },
}

impl CodexRefreshOutcome {
    pub fn into_blob(self) -> String {
        match self {
            Self::Unchanged { blob } | Self::Rotated { new_blob: blob } => blob,
        }
    }
}

/// The staleness rule and the OpenAI call, behind the store's refresh port.
struct CodexRefresher<'a> {
    http: &'a reqwest::Client,
    scope: &'a CodexScope,
}

#[async_trait]
impl CodexBlobRefresher for CodexRefresher<'_> {
    async fn refresh(&self, blob: &str) -> CodexBlobRefresh {
        let Ok(existing) = serde_json::from_str::<serde_json::Value>(blob) else {
            return CodexBlobRefresh::Unreadable;
        };

        if !codex_should_refresh(&existing) {
            debug!(
                scope = %self.scope.display(),
                "ensure_fresh_codex: token still fresh, skipping refresh"
            );
            return CodexBlobRefresh::Fresh;
        }

        let Some(refresh_token) = existing
            .get("tokens")
            .and_then(|t| t.get("refresh_token"))
            .and_then(serde_json::Value::as_str)
        else {
            return CodexBlobRefresh::Permanent {
                reason: "blob has no tokens.refresh_token".to_string(),
            };
        };

        info!(scope = %self.scope.display(), "ensure_fresh_codex: refreshing Codex tokens");
        match oauth::refresh_chatgpt_tokens(self.http, refresh_token).await {
            Ok(new_tokens) => CodexBlobRefresh::Rotated {
                blob: merge_refreshed_blob(&existing, &new_tokens).to_string(),
                refresh_token: new_tokens.refresh_token,
            },
            Err(oauth::RefreshTokenError::Permanent { reason, .. }) => {
                CodexBlobRefresh::Permanent {
                    reason: format!("{reason:?}"),
                }
            }
            Err(e) => {
                warn!(
                    scope = %self.scope.display(),
                    error = %e,
                    "ensure_fresh_codex: transient refresh failure, returning stale blob"
                );
                CodexBlobRefresh::Transient
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt_with_exp(exp_secs: i64) -> String {
        use base64::Engine as _;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
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

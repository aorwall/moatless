//! Persisting the GitHub App user access token a person authorized.
//!
//! Written by two flows that must agree byte for byte on the row they leave
//! behind: GitHub App *login* ([`crate::auth::sessions::create_session`]) and
//! *connect*, which attaches the same credential to a session established by
//! some other provider. A second copy of this would drift on the next change to
//! one of them.

use chrono::{DateTime, Utc};
use tracing::info;

use crate::auth::credentials::{
    GITHUB_OAUTH_STASH_KEY, StashedOAuthCredential, stash_oauth_credential,
};
use crate::auth::error::AuthError;
use crate::auth::models::{OAuthTokens, ProviderUserInfo};
use crate::auth::provider_token_auth_dao::{ProviderToken, ProviderTokenAuthDao};
use crate::auth::provider_tokens::AUTH_METHOD_APP_USER;
use crate::secret::service::SecretService;

/// What became of the freshly authorized token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persisted {
    /// The canonical GitHub row now carries this token.
    Credential,
    /// A PAT override holds the canonical row, so the token went to the stash
    /// [`crate::git_host_access::GitHostAccess::restore_github_app_token`]
    /// reads. Git operations keep using the PAT until the override is removed.
    StashedBehindPatOverride,
    /// GitHub returned no token, so nothing was written.
    NoToken,
}

/// Store `tokens` as `user_id`'s GitHub credential.
///
/// An active PAT override is not overwritten: the person set it deliberately
/// and re-authorizing the App is not a request to drop it. The stash is
/// refreshed instead, so removing the override later restores a live token
/// rather than an expired one.
///
/// A blank `access_token` is a no-op — GitHub returning no token is not a
/// reason to erase a working row.
pub async fn persist(
    provider_tokens: &dyn ProviderTokenAuthDao,
    secrets: &SecretService,
    user_id: &str,
    tokens: &OAuthTokens,
    user_info: &ProviderUserInfo,
) -> Result<Persisted, AuthError> {
    if tokens.access_token.is_empty() {
        return Ok(Persisted::NoToken);
    }
    let now = Utc::now();

    // Propagated, not defaulted: reading this wrong means taking the branch
    // below, and that branch overwrites the row a PAT override lives in.
    let overridden = provider_tokens
        .find_token_type(user_id, "github")
        .await?
        .as_deref()
        == Some("pat");

    if overridden {
        stash_oauth_credential(
            secrets,
            user_id,
            GITHUB_OAUTH_STASH_KEY,
            "github",
            &stashed(tokens, now),
        )
            .await
            .map_err(|e| AuthError::Internal(format!("stash the GitHub App token: {e}")))?;
        info!(
            user_id = %user_id,
            login = %user_info.login,
            "Updated stashed OAuth token (PAT override active)"
        );
        return Ok(Persisted::StashedBehindPatOverride);
    }

    provider_tokens
        .upsert_provider_token(&ProviderToken {
            user_id: user_id.to_string(),
            provider: "github".to_string(),
            auth_method: AUTH_METHOD_APP_USER.to_string(),
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            token_type: Some(tokens.token_type.clone()),
            scope: tokens.scope.clone(),
            provider_user_id: Some(user_info.id.clone()),
            provider_login: Some(user_info.login.clone()),
            connected_at: now,
            expires_at: expires_at(tokens, now),
        })
        .await?;
    info!(
        user_id = %user_id,
        login = %user_info.login,
        auth_method = %AUTH_METHOD_APP_USER,
        "Persisted GitHub token to provider_tokens"
    );
    Ok(Persisted::Credential)
}

/// The whole credential, so removing a PAT override later restores something
/// that can still be refreshed — see [`StashedOAuthCredential`].
fn stashed(tokens: &OAuthTokens, now: DateTime<Utc>) -> StashedOAuthCredential {
    StashedOAuthCredential {
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.clone(),
        expires_at: expires_at(tokens, now),
    }
}

/// When `tokens` stops working, from the TTL GitHub returned with them.
fn expires_at(tokens: &OAuthTokens, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    tokens
        .expires_in
        .map(|secs| now + chrono::Duration::seconds(secs as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::provider_token_auth_dao::MockProviderTokenAuthDao;
    use crate::secret::test_support::service_with;

    fn tokens(access_token: &str) -> OAuthTokens {
        OAuthTokens {
            access_token: access_token.to_string(),
            token_type: "bearer".to_string(),
            scope: None,
            id_token: None,
            refresh_token: Some("ghr_refresh".to_string()),
            expires_in: Some(28_800),
            refresh_token_expires_in: None,
        }
    }

    fn user_info() -> ProviderUserInfo {
        ProviderUserInfo {
            id: "4242".to_string(),
            login: "ada".to_string(),
            email: Some("ada@example.com".to_string()),
            name: None,
            avatar_url: None,
        }
    }

    #[tokio::test]
    async fn an_authorized_token_becomes_the_canonical_github_row() {
        let mut dao = MockProviderTokenAuthDao::new();
        dao.expect_find_token_type().returning(|_, _| Ok(None));
        dao.expect_upsert_provider_token().returning(|row| {
            assert_eq!(row.auth_method, AUTH_METHOD_APP_USER);
            assert_eq!(row.provider, "github");
            assert_eq!(row.access_token, "ghu_fresh");
            assert_eq!(row.refresh_token.as_deref(), Some("ghr_refresh"));
            assert_eq!(row.provider_login.as_deref(), Some("ada"));
            assert!(row.expires_at.is_some());
            Ok(())
        });

        let persisted = persist(
            &dao,
            &service_with(Vec::new()),
            "u1",
            &tokens("ghu_fresh"),
            &user_info(),
        )
        .await
        .expect("persist");

        assert_eq!(persisted, Persisted::Credential);
    }

    #[tokio::test]
    async fn a_pat_override_survives_reauthorizing_the_app() {
        let mut dao = MockProviderTokenAuthDao::new();
        dao.expect_find_token_type()
            .returning(|_, _| Ok(Some("pat".to_string())));
        // The override is the credential the person chose, so nothing is
        // written to the canonical row.
        dao.expect_upsert_provider_token().never();

        let secrets = service_with(Vec::new());
        let persisted = persist(&dao, &secrets, "u1", &tokens("ghu_fresh"), &user_info())
            .await
            .expect("persist");

        assert_eq!(persisted, Persisted::StashedBehindPatOverride);
        // The variant is only honest if the credential is really retrievable:
        // it is what removing the override hands back. The refresh token and
        // the expiry are part of that — an access token restored alone dies at
        // its own expiry with nothing able to renew it.
        let stashed = crate::auth::credentials::read_oauth_stash(&secrets, "u1", GITHUB_OAUTH_STASH_KEY)
            .await
            .expect("read the stash")
            .expect("a stashed credential");
        assert_eq!(stashed.access_token, "ghu_fresh");
        assert_eq!(stashed.refresh_token.as_deref(), Some("ghr_refresh"));
        assert!(stashed.expires_at.is_some());
    }

    #[tokio::test]
    async fn a_failed_override_read_writes_nothing() {
        // The alternative is treating the error as "no override" and taking the
        // branch below it, which overwrites the row a PAT override lives in.
        let mut dao = MockProviderTokenAuthDao::new();
        dao.expect_find_token_type()
            .returning(|_, _| Err(sqlx::Error::PoolClosed));
        dao.expect_upsert_provider_token().never();

        let error = persist(
            &dao,
            &service_with(Vec::new()),
            "u1",
            &tokens("ghu_fresh"),
            &user_info(),
        )
        .await
        .expect_err("a failed read is not a licence to overwrite");

        assert!(matches!(error, AuthError::Database(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn no_token_leaves_a_working_row_alone() {
        let mut dao = MockProviderTokenAuthDao::new();
        dao.expect_find_token_type().never();
        dao.expect_upsert_provider_token().never();

        let persisted = persist(
            &dao,
            &service_with(Vec::new()),
            "u1",
            &tokens(""),
            &user_info(),
        )
        .await
        .expect("persist");

        assert_eq!(persisted, Persisted::NoToken);
    }
}

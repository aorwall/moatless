use super::{
    error::AuthError,
    schemas::{ProviderConnection, SessionResponse, UserInfo},
    state::AuthState,
};
use crate::setup::{SetupContext, evaluate_setup};
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::{auth::AuthIdentity, user::UserRole};
use tracing::warn;

pub(super) async fn current_user(
    state: &AuthState,
    identity: AuthIdentity,
) -> Result<SessionResponse, AuthError> {
    // Fetch fresh user data from DB (email/name/avatar may have changed)
    let user = state.users.get_user(&identity.user_id).await?;
    let Some(user) = user else {
        return Err(AuthError::SessionExpired);
    };

    let setup_ctx = SetupContext {
        user_id: &identity.user_id.to_string(),
        is_admin: identity.role == UserRole::Admin,
        provider_tokens: state.provider_tokens_dao.as_ref(),
        secrets: Some(state.secrets.as_ref()),
    };
    let user_id_str = identity.user_id.to_string();
    let (providers, setup) = tokio::join!(
        check_provider_connections(state, &user_id_str),
        evaluate_setup(&setup_ctx),
    );

    Ok(SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: identity.provider,
            role: identity.role,
        }),
        expires_at: None,
        providers,
        setup: Some(setup),
    })
}

async fn check_provider_connections(state: &AuthState, user_id: &str) -> Vec<ProviderConnection> {
    let mut out = Vec::with_capacity(2);
    if let Some(github) = check_github_connection(state, user_id).await {
        out.push(github);
    }
    if let Some(codex) = check_codex_connection(state, user_id).await {
        out.push(codex);
    }
    out
}

/// Report the user's personal Codex credential in the `/me` providers array.
///
/// Returns `None` when the user has no codex row at all — most users never
/// connect Codex, and an absent entry keeps the frontend's `?? true` default
/// from showing a reconnect banner to them. When a row exists, `connected`
/// is false iff the refresher stamped `metadata.refresh_failure` (permanent
/// refresh failure — the user must sign in again).
async fn check_codex_connection(state: &AuthState, user_id: &str) -> Option<ProviderConnection> {
    let token = match state
        .provider_tokens_dao
        .find_provider_token_full(user_id, "codex")
        .await
    {
        Ok(row) => row?,
        Err(e) => {
            warn!(
                error = %e,
                user_id = %user_id,
                "Failed to load Codex provider token while building /me response"
            );
            return None;
        }
    };

    let metadata = token.metadata.as_ref();
    let needs_reconnect = metadata.is_some_and(|m| m.get("refresh_failure").is_some());
    let login = metadata
        .and_then(|m| m.get("email"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    Some(ProviderConnection {
        provider: "codex".to_string(),
        connected: !needs_reconnect,
        login,
    })
}

/// Returns `None` when the user has no GitHub token row at all — a user who
/// never connected GitHub is not "disconnected", and reporting them as such
/// pins a reconnect banner in the UI forever. Mirrors
/// [`check_codex_connection`].
async fn check_github_connection(state: &AuthState, user_id: &str) -> Option<ProviderConnection> {
    let disconnected = |login: Option<String>| {
        Some(ProviderConnection {
            provider: "github".to_string(),
            connected: false,
            login,
        })
    };

    let token = match state
        .provider_tokens_dao
        .find_provider_token_full(user_id, "github")
        .await
    {
        Ok(row) => row,
        Err(e) => {
            warn!(
                error = %e,
                user_id = %user_id,
                "Failed to load GitHub provider token while building /me response"
            );
            return disconnected(None);
        }
    };

    // No row → never connected, so report nothing rather than "disconnected".
    let token = token?;

    let login = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("login"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // When a GitHub App service is configured, verify the token by actually
    // resolving it. Any error (missing row, refresh rejected, dangling secret
    // reference) is reported as disconnected so the frontend prompts for a
    // reconnect.
    if state.github_app_service.is_some() {
        match state
            .git_host_tokens
            .github_is_usable(&moatless_types::access_control::Actor::of(user_id))
            .await
        {
            Ok(()) => Some(ProviderConnection {
                provider: "github".to_string(),
                connected: true,
                login,
            }),
            Err(e) => {
                warn!(
                    error = %e,
                    user_id = %user_id,
                    "GitHub token unavailable when reporting /me provider status"
                );
                disconnected(login)
            }
        }
    } else {
        // Without an App service we cannot refresh, so treat the stored token
        // as usable. Callers that actually hit GitHub will still surface 401s
        // through `RepositoryError::Forbidden` if the token has been revoked.
        Some(ProviderConnection {
            provider: "github".to_string(),
            connected: true,
            login,
        })
    }
}

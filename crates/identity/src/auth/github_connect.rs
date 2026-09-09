//! Attaching a GitHub App credential to a session that already exists.
//!
//! GitHub App *login* signs a person in as their GitHub identity. Connect runs
//! the same authorization flow and writes the same credential row, but issues
//! no session and revokes none: a deployment that signs people in with OIDC can
//! still give them GitHub access without a Personal Access Token.
//!
//! Both flows come back to the one callback URL registered on the App, so
//! whether a callback is a login or a connect is encoded in the CSRF state —
//! which means an attacker cannot flip one into the other without also
//! breaking the state/cookie match.

use axum::response::Response;
use tracing::warn;
use uuid::Uuid;

use super::connect_return::{approved_return_url, redirect, with_outcome};
use super::cookie;
use super::error::AuthError;
use super::github::user_credential::{self, Persisted};
use super::oauth::provider::OAuthProvider;
use super::routes::AuthState;

/// Separates the CSRF nonce and initiating user from the URL a finished connect
/// returns to.
const CONNECT_MARKER: &str = ":connect:";

/// Query parameter the return URL is loaded with, so the page that sent the
/// person to GitHub can report what happened.
const OUTCOME_PARAM: &str = "githubConnect";

/// What a connect callback carries back from the request that started it.
pub(super) struct ConnectState<'a> {
    /// Who asked for the credential. Compared against the session holding the
    /// callback, because the two need not be the same person — see
    /// [`finish`].
    pub user_id: Uuid,
    pub return_url: &'a str,
}

/// Encode the initiating user and `return_url` into `nonce`, so the callback
/// can tell a connect from a login, knows whose credential this is, and knows
/// where to send the browser afterwards.
pub(super) fn connect_state(nonce: &str, user_id: Uuid, return_url: &str) -> String {
    format!("{nonce}:{user_id}{CONNECT_MARKER}{return_url}")
}

/// The connect this `state` belongs to, or `None` when it came from a login.
pub(super) fn parse_connect_state(state: &str) -> Option<ConnectState<'_>> {
    let (head, return_url) = state.split_once(CONNECT_MARKER)?;
    // From the right: the nonce is opaque and a future one may contain `:`.
    let (_nonce, user_id) = head.rsplit_once(':')?;
    Some(ConnectState {
        user_id: user_id.parse().ok()?,
        return_url,
    })
}

/// Send the browser to GitHub to authorize the App on `user_id`'s behalf.
pub(super) fn start(
    state: &AuthState,
    provider: &dyn OAuthProvider,
    nonce: &str,
    user_id: Uuid,
    requested_return_url: Option<&str>,
) -> Result<Response, AuthError> {
    let return_url = approved_return_url(
        &state.config.auth.frontend_url,
        &state.config.auth.csrf_trusted_origins,
        requested_return_url,
    );
    let csrf_state = connect_state(nonce, user_id, &return_url);
    let auth_url = provider
        .generate_auth_url(&csrf_state)
        .ok_or_else(|| AuthError::Config("Failed to generate the GitHub App OAuth URL".into()))?;
    Ok(redirect(
        &auth_url,
        Some(cookie::build_oauth_state_cookie(&state.config, &csrf_state)),
    ))
}

/// Store the authorization `connect` asked for as its initiator's credential.
///
/// `viewer` is whoever holds the callback request, which is not necessarily who
/// started the flow: `oauth_state` outlives both a logout and the session
/// itself, so a consent screen left open past `session_max_age_secs`, or a
/// shared browser someone else signed into, both come back under a session that
/// is not the initiator's. `None` is that request carrying no session at all.
/// Writing the credential to the session in hand would hand a second person the
/// first person's GitHub account, so anything but a match stores nothing.
///
/// `code` is absent when the person declined on GitHub.
///
/// Always redirects to the return URL, carrying the outcome as a query
/// parameter: the browser is mid-flow on a page the person opened, and a JSON
/// error body would strand them there.
pub(super) async fn finish(
    state: &AuthState,
    provider: &dyn OAuthProvider,
    viewer: Option<Uuid>,
    code: Option<&str>,
    connect_state: &ConnectState<'_>,
) -> Response {
    let clear_state = cookie::build_clear_oauth_state_cookie(&state.config);
    // Re-vetted rather than trusted from the state, so this does not depend on
    // `oauth_state` only ever having been set by `start`.
    let return_url = approved_return_url(
        &state.config.auth.frontend_url,
        &state.config.auth.csrf_trusted_origins,
        Some(connect_state.return_url),
    );
    let user_id = connect_state.user_id;

    let outcome = if viewer != Some(user_id) {
        warn!(
            %user_id, ?viewer,
            "Discarded a GitHub App connect: the session changed or expired while it was in flight"
        );
        "session_changed"
    } else if let Some(code) = code {
        match connect(state, provider, user_id, code).await {
            Ok(Persisted::Credential) => "connected",
            // The person's PAT still decides every git operation, so reporting
            // "connected" here would describe a credential that is not in use.
            Ok(Persisted::StashedBehindPatOverride) => "pat_override",
            Ok(Persisted::NoToken) => "empty_token",
            Err(e) => {
                warn!(error = %e, %user_id, "GitHub App connect failed");
                "error"
            }
        }
    } else {
        "denied"
    };
    redirect(
        &with_outcome(&return_url, OUTCOME_PARAM, outcome),
        Some(clear_state),
    )
}

async fn connect(
    state: &AuthState,
    provider: &dyn OAuthProvider,
    user_id: Uuid,
    code: &str,
) -> Result<Persisted, AuthError> {
    let tokens = provider.exchange_code_for_tokens(code).await?;
    let user_info = provider.get_user_info(&tokens).await?;

    let persisted = user_credential::persist(
        state.provider_tokens.as_ref(),
        state.secrets.as_ref(),
        &user_id.to_string(),
        &tokens,
        &user_info,
    )
    .await?;

    // Lets an inbound GitHub event resolve this person. Not worth failing a
    // connect over: the credential is written and usable without it.
    if let Err(e) = state
        .users
        .upsert_provider_id(&user_id, "github", &user_info.id)
        .await
    {
        warn!(error = %e, %user_id, "Failed to record the GitHub provider identity");
    }

    Ok(persisted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Uuid {
        Uuid::parse_str("11111111-2222-3333-4444-555555555555").expect("a valid uuid")
    }

    #[test]
    fn a_login_state_is_not_mistaken_for_a_connect() {
        assert!(parse_connect_state("deadbeef").is_none());
        assert!(parse_connect_state("deadbeef:cli:1234").is_none());
    }

    #[test]
    fn a_connect_state_round_trips_its_user_and_return_url() {
        let state = connect_state("deadbeef", user(), "https://t3.example/settings/account");
        let parsed = parse_connect_state(&state).expect("a connect state");
        assert_eq!(parsed.user_id, user());
        assert_eq!(parsed.return_url, "https://t3.example/settings/account");
    }

    #[test]
    fn a_state_without_a_usable_user_id_is_not_a_connect() {
        // Nothing may be written on a guess about whose credential this is, so
        // an unparseable owner drops the callback rather than defaulting one.
        assert!(parse_connect_state("deadbeef:connect:https://t3.example/s").is_none());
        assert!(parse_connect_state("deadbeef:not-a-uuid:connect:https://t3.example/s").is_none());
    }

    #[test]
    fn a_return_url_that_keeps_its_own_query_still_round_trips() {
        // The URL is the whole tail of the state, so a `?` or a second `:` in
        // it is not a delimiter.
        let state = connect_state("deadbeef", user(), "https://t3.example:8443/s?tab=git");
        assert_eq!(
            parse_connect_state(&state)
                .expect("a connect state")
                .return_url,
            "https://t3.example:8443/s?tab=git"
        );
    }
}

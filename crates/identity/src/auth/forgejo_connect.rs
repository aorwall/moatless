//! Attaching a Forgejo OAuth credential to a session that already exists.
//!
//! The GitHub counterpart is [`super::github_connect`], and both write a
//! credential row without issuing or revoking a session. What differs is
//! addressing: Forgejo is self-hostable, so the host selects the registered
//! OAuth2 application, and it rides in the CSRF state so the callback writes
//! against the instance the flow started on.
//!
//! Forgejo has no login flow here, so this gets its own callback route rather
//! than sharing one — a `state` that does not parse is simply not a connect.

use axum::response::Response;
use moatless_types::access_control::ScopeContext;
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;
use tracing::warn;
use uuid::Uuid;

use super::connect_return::{approved_return_url, redirect, with_outcome};
use super::cookie;
use super::error::AuthError;
use super::state::AuthState;
use crate::forgejo::{callback_url, normalize_host};
use crate::git_host_access::{GitHostAccessError, GitHostPrincipal};

/// Query parameter the return URL is loaded with, so the page that sent the
/// person to Forgejo can report what happened.
const OUTCOME_PARAM: &str = "forgejoConnect";

/// Separates the CSRF head, the host and the return URL in the `state`.
///
/// Not `:`, which a nonce, a `host:port` and a URL all contain. A Forgejo host
/// cannot contain it — `forgejo_instances_host_format_chk` restricts one to
/// `[a-z0-9.-]` and an optional numeric port.
const FIELD: char = '|';

/// What a connect callback carries back from the request that started it.
pub(super) struct ConnectState<'a> {
    /// Who asked for the credential. Compared against the session holding the
    /// callback, because the two need not be the same person — see [`finish`].
    pub user_id: Uuid,
    /// The instance the credential belongs to, already normalized by [`start`].
    pub host: &'a str,
    pub return_url: &'a str,
}

fn connect_state(nonce: &str, user_id: Uuid, host: &str, return_url: &str) -> String {
    format!("{nonce}:{user_id}{FIELD}{host}{FIELD}{return_url}")
}

/// The connect this `state` belongs to, or `None` when it is not one.
pub(super) fn parse_connect_state(state: &str) -> Option<ConnectState<'_>> {
    let (head, tail) = state.split_once(FIELD)?;
    // From the right: the nonce is opaque and a future one may contain `:`.
    let (_nonce, user_id) = head.rsplit_once(':')?;
    let (host, return_url) = tail.split_once(FIELD)?;
    Some(ConnectState {
        user_id: user_id.parse().ok()?,
        host,
        return_url,
    })
}

/// Send the browser to `host` to authorize this deployment on `user_id`'s
/// behalf.
///
/// `NotFound` when `host` has no registered OAuth2 application, which leaves a
/// personal access token as the only way to connect it.
pub(super) async fn start(
    state: &AuthState,
    nonce: &str,
    user_id: Uuid,
    host: &str,
    requested_return_url: Option<&str>,
) -> Result<Response, AuthError> {
    let host = normalize_host(host);
    let return_url = approved_return_url(
        &state.config.auth.frontend_url,
        &state.config.auth.csrf_trusted_origins,
        requested_return_url,
    );
    let csrf_state = connect_state(nonce, user_id, &host, &return_url);
    let auth_url = state
        .forgejo_access
        .authorize_url(
            &host,
            &callback_url(&state.config.auth.frontend_url),
            &csrf_state,
        )
        .await
        .map_err(access_error)?;
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
/// first person's Forgejo account, so anything but a match stores nothing.
///
/// `code` is absent when the person declined on Forgejo.
///
/// Always redirects to the return URL, carrying the outcome as a query
/// parameter: the browser is mid-flow on a page the person opened, and a JSON
/// error body would strand them there.
pub(super) async fn finish(
    state: &AuthState,
    viewer: Option<&AuthIdentity>,
    code: Option<&str>,
    connect: &ConnectState<'_>,
) -> Response {
    let clear_state = cookie::build_clear_oauth_state_cookie(&state.config);
    // Re-vetted rather than trusted from the state, so this does not depend on
    // `oauth_state` only ever having been set by `start`.
    let return_url = approved_return_url(
        &state.config.auth.frontend_url,
        &state.config.auth.csrf_trusted_origins,
        Some(connect.return_url),
    );
    let user_id = connect.user_id;

    let viewer = viewer.filter(|identity| identity.user_id == user_id);
    let outcome = match (viewer, code) {
        (None, _) => {
            warn!(
                %user_id,
                "Discarded a Forgejo connect: the session changed or expired while it was in flight"
            );
            "session_changed"
        }
        (Some(_), None) => "denied",
        (Some(identity), Some(code)) => {
            match state
                .forgejo_access
                .complete_connect(
                    &principal(identity),
                    connect.host,
                    code,
                    &callback_url(&state.config.auth.frontend_url),
                )
                .await
            {
                // A personal access token still decides every git operation, so
                // reporting "connected" would describe a credential not in use.
                Ok(status) if status.has_pat_override => "pat_override",
                Ok(_) => "connected",
                Err(e) => {
                    warn!(error = %e, %user_id, host = connect.host, "Forgejo connect failed");
                    "error"
                }
            }
        }
    };
    redirect(
        &with_outcome(&return_url, OUTCOME_PARAM, outcome),
        Some(clear_state),
    )
}

fn principal(identity: &AuthIdentity) -> GitHostPrincipal {
    GitHostPrincipal {
        user_id: identity.user_id.to_string(),
        scope_context: ScopeContext {
            user_id: identity.user_id.to_string(),
            is_admin: identity.role == UserRole::Admin,
        },
    }
}

/// A host with no registered application is the one failure a screen acts on —
/// it offers a personal access token instead — so it keeps its own status.
fn access_error(error: GitHostAccessError) -> AuthError {
    match error {
        GitHostAccessError::NotFound(message) => AuthError::NotFound(message),
        other => AuthError::OAuth(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Uuid {
        Uuid::parse_str("11111111-2222-3333-4444-555555555555").expect("a valid uuid")
    }

    #[test]
    fn a_state_round_trips_its_user_host_and_return_url() {
        let state = connect_state(
            "deadbeef",
            user(),
            "git.acme.dev:3000",
            "https://t3.example/settings/account?tab=git",
        );
        let parsed = parse_connect_state(&state).expect("a connect state");
        assert_eq!(parsed.user_id, user());
        // The port is part of the host, and `:` is therefore not a delimiter.
        assert_eq!(parsed.host, "git.acme.dev:3000");
        assert_eq!(
            parsed.return_url,
            "https://t3.example/settings/account?tab=git"
        );
    }

    #[test]
    fn a_state_from_another_flow_is_not_a_forgejo_connect() {
        assert!(parse_connect_state("deadbeef").is_none());
        assert!(parse_connect_state("deadbeef:cli:1234").is_none());
        // A GitHub connect state: it has the user id, but no host field.
        assert!(
            parse_connect_state(
                "deadbeef:11111111-2222-3333-4444-555555555555:connect:https://t3.example/s"
            )
            .is_none()
        );
    }

    #[test]
    fn a_state_without_a_usable_user_id_is_dropped() {
        // Nothing may be written on a guess about whose credential this is, so
        // an unparseable owner drops the callback rather than defaulting one.
        assert!(parse_connect_state("deadbeef|git.acme.dev|https://t3.example/s").is_none());
        assert!(
            parse_connect_state("deadbeef:not-a-uuid|git.acme.dev|https://t3.example/s").is_none()
        );
    }
}

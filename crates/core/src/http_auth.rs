//! Request extractors for authentication.
//!
//! Axum plumbing over two consumer-declared ports, so a crate that mounts
//! routes gets the extractors without naming the identity band.
//!
//! Two auth paths, tried in order: the `AuthIdentity` extension set by auth
//! middleware after validating a session cookie or API key, then the
//! `X-User-Token` header (sandbox→backend, JWT-based) for requests that bypass
//! the middleware. Anything else is a 401.
//!
//! `X-User-*` headers are never trusted — the auth middleware strips them
//! before validation.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use moatless_types::access_control::ScopeContext;
use moatless_types::auth::{AuthIdentity, AuthMethod};
use moatless_types::user::UserRole;

/// Verifies a sandbox-issued `X-User-Token` JWT.
///
/// Declared consumer-side so the extractors name only this one value from the
/// process rather than the whole `AppState`; the implementation (with its
/// `sandbox::jwt` and token-secret dependency) lives behind this port, so
/// generifying the extractors over any router state `S` costs no `sandbox`
/// import here.
pub trait UserTokenVerifier: Send + Sync {
    /// Verify a long-lived user auth token, returning the identity it carries.
    fn verify_user_token(&self, token: &str) -> Result<VerifiedUserToken, TokenVerifyError>;
}

/// The identity a verified user token carries. Only the two fields the auth
/// extractor reads.
pub struct VerifiedUserToken {
    pub user_id: String,
    pub login: String,
}

/// Why verifying a user token failed.
#[derive(Debug)]
pub enum TokenVerifyError {
    /// This process was started without a token secret, so no user token can be
    /// verified.
    NotConfigured,
    /// A token was present but did not verify.
    Invalid(String),
}

/// Whether the authenticated caller may reach a task's preview URL — the one
/// authorization bit the `ForwardAuth` session gate needs.
///
/// Declared here beside the extractors so the `ForwardAuth` handler holds this
/// bool port rather than a `TaskDao`: the implementation over `TaskDao` lives
/// in the tasks band, behind this port.
#[async_trait]
pub trait TaskAccess: Send + Sync {
    /// True iff the task exists and `ctx` is allowed to see it. Fail-closed: an
    /// implementation denies (returns `false`) on a lookup error.
    async fn may_access_task(&self, task_id: &str, ctx: &ScopeContext) -> bool;
}

/// Authenticated user extracted from request extensions, user token, or
/// dev-mode headers.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub login: String,
    pub role: Option<String>,
    pub provider: Option<String>,
    /// See [`crate::auth::jwt::SessionClaims::group_id`].
    pub group_id: Option<String>,
    pub task_id: Option<String>,
    /// True when this request was authenticated via the sandbox-issued
    /// `X-User-Token` JWT (i.e. came from inside a sandbox pod, on behalf of
    /// the agent's CLI). Used to mark feedback rows as `source = 'agent'`.
    pub is_agent_context: bool,
}

impl AuthUser {
    /// Build a `ScopeContext` from this authenticated user.
    pub fn scope_context(&self) -> ScopeContext {
        ScopeContext {
            user_id: self.user_id.to_string(),
            is_admin: self.role.as_deref() == Some("admin"),
        }
    }
}

/// Error returned when auth headers are missing.
#[derive(Debug)]
pub struct AuthError(String);

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, self.0).into_response()
    }
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Read X-Task-Id header (used by all auth paths)
        let task_id = parts
            .headers
            .get("x-task-id")
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);

        // Path 1: AuthIdentity from middleware (production — auth provider configured)
        if let Some(identity) = parts.extensions.get::<AuthIdentity>() {
            // The middleware authenticates a sandbox X-User-Token to an identity
            // just like any other credential; the method it stamped alongside is
            // what marks the request as agent context.
            let is_agent_context = matches!(
                parts.extensions.get::<AuthMethod>(),
                Some(AuthMethod::UserToken)
            );
            return Ok(AuthUser {
                user_id: identity.user_id,
                login: identity.login.clone(),
                role: Some(identity.role.as_str().to_string()),
                provider: Some(identity.provider.clone()),
                group_id: identity.group_id.clone(),
                task_id,
                is_agent_context,
            });
        }

        // Path 2: X-User-Token header (sandbox→backend JWT auth — bypasses middleware)
        if let Some(token) = parts
            .headers
            .get("x-user-token")
            .and_then(|v| v.to_str().ok())
        {
            let verifier = Arc::<dyn UserTokenVerifier>::from_ref(state);
            let verified = verifier.verify_user_token(token).map_err(|e| match e {
                TokenVerifyError::NotConfigured => {
                    AuthError("Token verification not configured".to_string())
                }
                TokenVerifyError::Invalid(msg) => AuthError(format!("Invalid user token: {msg}")),
            })?;

            let user_id = verified
                .user_id
                .parse::<Uuid>()
                .map_err(|_| AuthError("Invalid user id in token".to_string()))?;

            return Ok(AuthUser {
                user_id,
                login: verified.login,
                role: None,
                provider: Some("user-token".to_string()),
                group_id: None,
                task_id,
                is_agent_context: true,
            });
        }

        // No valid auth found
        Err(AuthError("Authentication required".to_string()))
    }
}

/// Optional auth extractor for endpoints that tolerate anonymous callers.
///
/// Returns `None` instead of 401 when the request did not authenticate; callers
/// must treat `None` as a denial, never as an unscoped admin.
#[derive(Debug, Clone)]
pub struct MaybeAuthUser(pub Option<AuthUser>);

impl<S> FromRequestParts<S> for MaybeAuthUser
where
    S: Send + Sync,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeAuthUser(Some(user))),
            Err(_) => Ok(MaybeAuthUser(None)),
        }
    }
}

/// State-agnostic optional-scope extractor for routers whose state is **not**
/// `AppState` (e.g. the sandbox router's `SandboxApiState`).
///
/// Reads the canonical [`AuthIdentity`] that `auth_middleware` injects,
/// trusting only that middleware-set extension (never raw `X-User-*` headers);
/// `None` means no identity was attached, and callers must treat it as a
/// denial, never an unscoped admin.
#[derive(Debug, Clone)]
pub struct OptionalScope(pub Option<ScopeContext>);

impl<S: Send + Sync> FromRequestParts<S> for OptionalScope {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthIdentity>()
            .map(|id| ScopeContext {
                user_id: id.user_id.to_string(),
                is_admin: id.role == UserRole::Admin,
            });
        Ok(OptionalScope(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    /// Configurable [`UserTokenVerifier`] for exercising the `X-User-Token`
    /// path of the auth extractor without minting real JWTs — the second
    /// implementor of the port. `verify_user_auth_token` itself is covered in
    /// `sandbox::jwt`.
    enum StubOutcome {
        Ok { user_id: String, login: String },
        NotConfigured,
        Invalid(String),
    }

    struct StubVerifier(StubOutcome);

    impl UserTokenVerifier for StubVerifier {
        fn verify_user_token(&self, _token: &str) -> Result<VerifiedUserToken, TokenVerifyError> {
            match &self.0 {
                StubOutcome::Ok { user_id, login } => Ok(VerifiedUserToken {
                    user_id: user_id.clone(),
                    login: login.clone(),
                }),
                StubOutcome::NotConfigured => Err(TokenVerifyError::NotConfigured),
                StubOutcome::Invalid(msg) => Err(TokenVerifyError::Invalid(msg.clone())),
            }
        }
    }

    // A bare `Arc<dyn UserTokenVerifier>` is a valid router state here: axum's
    // blanket `FromRef<S> for S` (S: Clone) satisfies the extractor's bound.
    fn state(outcome: StubOutcome) -> Arc<dyn UserTokenVerifier> {
        Arc::new(StubVerifier(outcome))
    }

    fn parts_with_token(token: Option<&str>, task_id: Option<&str>) -> Parts {
        let mut builder = Request::builder();
        if let Some(token) = token {
            builder = builder.header("x-user-token", token);
        }
        if let Some(task_id) = task_id {
            builder = builder.header("x-task-id", task_id);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[tokio::test]
    async fn valid_token_authenticates_as_agent_context() {
        let uid = "11111111-1111-1111-1111-111111111111";
        let st = state(StubOutcome::Ok {
            user_id: uid.to_string(),
            login: "alice".to_string(),
        });
        let mut parts = parts_with_token(Some("tok"), Some("task-42"));

        let user = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect("valid token authenticates");

        assert_eq!(user.user_id.to_string(), uid);
        assert_eq!(user.login, "alice");
        assert_eq!(user.provider.as_deref(), Some("user-token"));
        assert_eq!(user.task_id.as_deref(), Some("task-42"));
        assert!(user.is_agent_context);
    }

    fn parts_with_identity(method: AuthMethod) -> Parts {
        let mut parts = Request::builder().body(()).unwrap().into_parts().0;
        parts.extensions.insert(AuthIdentity {
            user_id: Uuid::nil(),
            login: "alice".to_string(),
            role: UserRole::User,
            provider: "user-token".to_string(),
            group_id: None,
        });
        parts.extensions.insert(method);
        parts
    }

    #[tokio::test]
    async fn middleware_user_token_identity_is_agent_context() {
        // The middleware authenticated a sandbox X-User-Token: identity present,
        // method stamped as UserToken. The extractor must carry the agent mark.
        let st = state(StubOutcome::NotConfigured);
        let mut parts = parts_with_identity(AuthMethod::UserToken);

        let user = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect("identity authenticates");
        assert!(user.is_agent_context);
    }

    #[tokio::test]
    async fn middleware_cookie_identity_is_not_agent_context() {
        let st = state(StubOutcome::NotConfigured);
        let mut parts = parts_with_identity(AuthMethod::SessionCookie);

        let user = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect("identity authenticates");
        assert!(!user.is_agent_context);
    }

    #[tokio::test]
    async fn missing_secret_maps_to_not_configured_rejection() {
        let st = state(StubOutcome::NotConfigured);
        let mut parts = parts_with_token(Some("tok"), None);

        let err = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect_err("no secret rejects");
        assert_eq!(err.0, "Token verification not configured");
    }

    #[tokio::test]
    async fn invalid_token_maps_to_invalid_rejection() {
        let st = state(StubOutcome::Invalid("bad signature".to_string()));
        let mut parts = parts_with_token(Some("tok"), None);

        let err = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect_err("invalid token rejects");
        assert_eq!(err.0, "Invalid user token: bad signature");
    }

    #[tokio::test]
    async fn non_uuid_user_id_is_rejected() {
        let st = state(StubOutcome::Ok {
            user_id: "not-a-uuid".to_string(),
            login: "alice".to_string(),
        });
        let mut parts = parts_with_token(Some("tok"), None);

        let err = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect_err("non-uuid rejects");
        assert_eq!(err.0, "Invalid user id in token");
    }

    #[tokio::test]
    async fn no_token_and_no_identity_falls_through() {
        let st = state(StubOutcome::NotConfigured);
        let mut parts = parts_with_token(None, None);

        let err = AuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect_err("no auth rejects");
        assert_eq!(err.0, "Authentication required");
    }

    #[tokio::test]
    async fn maybe_auth_user_swallows_rejection() {
        let st = state(StubOutcome::Invalid("bad".to_string()));
        let mut parts = parts_with_token(Some("tok"), None);

        let MaybeAuthUser(user) = MaybeAuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect("MaybeAuthUser is infallible");
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn maybe_auth_user_passes_through_success() {
        let st = state(StubOutcome::Ok {
            user_id: "22222222-2222-2222-2222-222222222222".to_string(),
            login: "bob".to_string(),
        });
        let mut parts = parts_with_token(Some("tok"), None);

        let MaybeAuthUser(user) = MaybeAuthUser::from_request_parts(&mut parts, &st)
            .await
            .expect("MaybeAuthUser is infallible");
        assert_eq!(user.expect("some user").login, "bob");
    }
}

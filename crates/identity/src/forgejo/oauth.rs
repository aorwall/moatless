//! Forgejo's OAuth2 authorization-code flow, run against one instance.
//!
//! Forgejo grants no scopes on an OAuth2 token: the credential reaches
//! everything its owner reaches. The scoped alternative is a personal access
//! token, which `crate::forgejo::access` stores instead.

use chrono::{DateTime, Duration, Utc};
use moatless_core::git_provider::GitProviderError;
use serde::Deserialize;

use crate::forgejo::instances::{ForgejoOAuthApp, base_url};

const OPERATION: &str = "Forgejo";

/// The authorization code [`ForgejoOAuth::verify_client_credentials`] presents.
/// Forgejo has to reject it, and what it says while rejecting it is the answer.
const PROBE_CODE: &str = "moatless-credential-probe";

/// The `error_description` fragments Forgejo answers `unauthorized_client` with
/// when the client credentials themselves are wrong, lowercased for comparison.
///
/// From `handleAuthorizationCode` (`routers/web/auth/oauth.go`): "invalid client
/// secret", "invalid empty client secret" and "unexpected redirect URI".
const CLIENT_REJECTIONS: [&str; 2] = ["client secret", "unexpected redirect uri"];

/// A credential Forgejo issued, and when it stops working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgejoOAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Forgejo's `/login/oauth` endpoints.
pub struct ForgejoOAuth {
    client: reqwest::Client,
}

impl Default for ForgejoOAuth {
    fn default() -> Self {
        Self {
            client: super::http_client(),
        }
    }
}

impl ForgejoOAuth {
    /// Where to send somebody to authorize `app`.
    ///
    /// No `scope` parameter: Forgejo's OAuth2 has none, and sending one is not
    /// a narrower grant but a rejected request.
    pub fn authorize_url(app: &ForgejoOAuthApp, redirect_uri: &str, state: &str) -> String {
        let base = base_url(&app.host);
        let client_id = urlencoding::encode(&app.client_id);
        let redirect = urlencoding::encode(redirect_uri);
        let state = urlencoding::encode(state);
        format!(
            "{base}/login/oauth/authorize?client_id={client_id}&redirect_uri={redirect}\
             &response_type=code&state={state}"
        )
    }

    /// Trade an authorization code for a credential.
    ///
    /// `redirect_uri` must be the one the authorization request carried;
    /// Forgejo compares them.
    pub async fn exchange_code(
        &self,
        app: &ForgejoOAuthApp,
        code: &str,
        redirect_uri: &str,
    ) -> Result<ForgejoOAuthTokens, GitProviderError> {
        self.post_token(
            app,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &app.client_id),
                ("client_secret", &app.client_secret),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ],
        )
        .await
    }

    /// Check `app`'s client id, client secret and `redirect_uri` against its
    /// instance, with nobody authorizing anything.
    ///
    /// Forgejo validates all three before it looks an authorization code up, so
    /// a code it is bound to reject still says which of them is wrong. Returns
    /// that rejection unchanged.
    ///
    /// A public client proves only its id: Forgejo checks a secret only when
    /// the application sets `ConfidentialClient`.
    pub async fn verify_client_credentials(
        &self,
        app: &ForgejoOAuthApp,
        redirect_uri: &str,
    ) -> Result<(), GitProviderError> {
        match self.exchange_code(app, PROBE_CODE, redirect_uri).await {
            Ok(_) => Ok(()),
            Err(error) if only_the_code_was_rejected(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Exchange a refresh token for a fresh credential.
    ///
    /// Forgejo rotates the refresh token on every exchange and invalidates the
    /// one presented, so the answer must be stored whole.
    pub async fn refresh(
        &self,
        app: &ForgejoOAuthApp,
        refresh_token: &str,
    ) -> Result<ForgejoOAuthTokens, GitProviderError> {
        self.post_token(
            app,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &app.client_id),
                ("client_secret", &app.client_secret),
                ("refresh_token", refresh_token),
            ],
        )
        .await
    }

    async fn post_token(
        &self,
        app: &ForgejoOAuthApp,
        form: &[(&str, &str)],
    ) -> Result<ForgejoOAuthTokens, GitProviderError> {
        let base = base_url(&app.host);
        let response = self
            .client
            .post(format!("{base}/login/oauth/access_token"))
            .header("Accept", "application/json")
            .form(form)
            .send()
            .await
            .map_err(|source| GitProviderError::Unreachable {
                operation: OPERATION,
                source,
            })?;

        let status = response.status();
        let body: TokenResponse =
            response
                .json()
                .await
                .map_err(|source| GitProviderError::UndecodableResponse {
                    operation: OPERATION,
                    source,
                })?;

        // Forgejo answers a bad client or a spent code with a JSON `error`
        // body, under 4xx as well as 200 — read the payload before the status,
        // so the reason reaches the caller instead of a bare status line.
        if let Some(code) = body.error {
            return Err(GitProviderError::OAuthRejected {
                provider: OPERATION,
                code,
                description: body.error_description.unwrap_or_default(),
            });
        }
        let Some(access_token) = body.access_token.filter(|token| !token.is_empty()) else {
            return Err(GitProviderError::MalformedResponse(format!(
                "Forgejo answered {status} with no access_token"
            )));
        };

        Ok(ForgejoOAuthTokens {
            access_token,
            refresh_token: body.refresh_token.filter(|token| !token.is_empty()),
            expires_at: body
                .expires_in
                .and_then(|seconds| Utc::now().checked_add_signed(Duration::seconds(seconds))),
        })
    }
}

/// Whether `error` is Forgejo rejecting [`PROBE_CODE`] alone, having accepted
/// the client id, the client secret and the redirect URI before it reached the
/// code.
///
/// Wording under `unauthorized_client` that matches no [`CLIENT_REJECTIONS`]
/// entry counts as the code: a description a later Forgejo release rewords then
/// still registers, rather than locking every admin out.
fn only_the_code_was_rejected(error: &GitProviderError) -> bool {
    let GitProviderError::OAuthRejected {
        code, description, ..
    } = error
    else {
        return false;
    };
    match code.as_str() {
        "invalid_grant" => true,
        "unauthorized_client" => {
            let description = description.to_ascii_lowercase();
            !CLIENT_REJECTIONS
                .iter()
                .any(|fragment| description.contains(fragment))
        }
        _ => false,
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> ForgejoOAuthApp {
        ForgejoOAuthApp {
            host: "git.acme.dev".to_string(),
            client_id: "cid".to_string(),
            client_secret: "shh".to_string(),
        }
    }

    #[test]
    fn an_authorize_url_carries_no_scope() {
        // Forgejo's OAuth2 has no scopes, and sending one is a rejected
        // request rather than a narrower grant.
        let url = ForgejoOAuth::authorize_url(&app(), "https://moatless.dev/cb", "st8");
        assert!(!url.contains("scope"), "{url}");
        assert!(
            url.starts_with("https://git.acme.dev/login/oauth/authorize?"),
            "{url}"
        );
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fmoatless.dev%2Fcb"),
            "{url}"
        );
        assert!(url.contains("state=st8"), "{url}");
    }

    fn rejected(code: &str, description: &str) -> GitProviderError {
        GitProviderError::OAuthRejected {
            provider: OPERATION,
            code: code.to_string(),
            description: description.to_string(),
        }
    }

    #[test]
    fn a_rejected_client_is_told_apart_from_a_rejected_code() {
        // The three `handleAuthorizationCode` checks that run before the code.
        assert!(!only_the_code_was_rejected(&rejected(
            "invalid_client",
            "cannot load client with client id: 'nope'"
        )));
        assert!(!only_the_code_was_rejected(&rejected(
            "unauthorized_client",
            "invalid client secret"
        )));
        assert!(!only_the_code_was_rejected(&rejected(
            "unauthorized_client",
            "invalid empty client secret"
        )));
        assert!(!only_the_code_was_rejected(&rejected(
            "unauthorized_client",
            "unexpected redirect URI"
        )));

        // Past all three: the probe code is the only thing left to fail on.
        assert!(only_the_code_was_rejected(&rejected(
            "unauthorized_client",
            "client is not authorized"
        )));
        assert!(only_the_code_was_rejected(&rejected("invalid_grant", "")));
        // Reworded by a later release, and still registered.
        assert!(only_the_code_was_rejected(&rejected(
            "unauthorized_client",
            "no such authorization"
        )));

        assert!(!only_the_code_was_rejected(
            &GitProviderError::MalformedResponse(
                "Forgejo answered 200 with no access_token".into()
            )
        ));
    }
}

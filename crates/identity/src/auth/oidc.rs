//! Standards-based OpenID Connect login.
//!
//! OIDC identities are keyed by `(issuer, subject)`. A verified email is used
//! only when policy permits first-login linking or provisioning; subsequent
//! logins never depend on the email claim.

use chrono::{Duration, Utc};
use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreRevocableToken, CoreRevocationErrorResponse,
    CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AdditionalClaims, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken,
    EmptyExtraTokenFields, EndpointNotSet, IdTokenClaims, IdTokenFields, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, StandardErrorResponse,
    StandardTokenResponse, TokenResponse,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use tokio::sync::RwLock;
use uuid::Uuid;

use moatless_core::config::OidcConfig;
use moatless_core::db::DbPool;

use super::error::AuthError;
use super::models::UserRole;
use super::users::User;

const TRANSACTION_TTL_MINUTES: i64 = 5;
const GOOGLE_ISSUER: &str = "https://accounts.google.com";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct OidcAdditionalClaims {
    /// Google Workspace domain, signed into Google's ID token as `hd`.
    #[serde(default, rename = "hd")]
    hosted_domain: Option<String>,
}

impl AdditionalClaims for OidcAdditionalClaims {}

type OidcIdTokenFields = IdTokenFields<
    OidcAdditionalClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type OidcTokenResponse = StandardTokenResponse<OidcIdTokenFields, CoreTokenType>;
type OidcClient<
    HasAuthUrl = EndpointNotSet,
    HasDeviceAuthUrl = EndpointNotSet,
    HasIntrospectionUrl = EndpointNotSet,
    HasRevocationUrl = EndpointNotSet,
    HasTokenUrl = EndpointNotSet,
    HasUserInfoUrl = EndpointNotSet,
> = Client<
    OidcAdditionalClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    OidcTokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
    HasUserInfoUrl,
>;

struct OidcIdentity {
    issuer: String,
    subject: String,
    email: Option<String>,
    email_verified: bool,
    hosted_domain: Option<String>,
}

#[derive(FromRow)]
struct AuthTransaction {
    nonce: String,
    pkce_verifier: String,
    redirect_after_login: Option<String>,
    cli_callback_port: Option<i32>,
}

/// Result of a verified OIDC callback.
pub struct OidcAuthentication {
    pub user: User,
    pub redirect_after_login: Option<String>,
    pub cli_callback_port: Option<i32>,
}

/// Concrete OIDC protocol and account-policy boundary.
pub struct OidcService {
    config: OidcConfig,
    db: DbPool,
    http: openidconnect::reqwest::Client,
    metadata: RwLock<Option<CoreProviderMetadata>>,
}

impl OidcService {
    pub fn new(config: OidcConfig, db: DbPool) -> Result<Self, AuthError> {
        let http = openidconnect::reqwest::ClientBuilder::new()
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| AuthError::Config(format!("OIDC HTTP client: {error}")))?;
        Ok(Self {
            config,
            db,
            http,
            metadata: RwLock::new(None),
        })
    }

    pub fn provider_id(&self) -> &str {
        &self.config.provider_id
    }

    async fn metadata(&self) -> Result<CoreProviderMetadata, AuthError> {
        if let Some(metadata) = self.metadata.read().await.clone() {
            return Ok(metadata);
        }

        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(self.config.issuer.clone())
                .map_err(|error| AuthError::Config(format!("OIDC issuer: {error}")))?,
            &self.http,
        )
        .await
        .map_err(|error| AuthError::OAuth(format!("OIDC discovery failed: {error}")))?;
        *self.metadata.write().await = Some(metadata.clone());
        Ok(metadata)
    }

    fn redirect_url(&self) -> Result<RedirectUrl, AuthError> {
        RedirectUrl::new(self.config.callback_url.clone())
            .map_err(|error| AuthError::Config(format!("OIDC callback URL: {error}")))
    }

    /// Create a one-time server-side transaction and return the provider URL.
    pub async fn start(
        &self,
        redirect_after_login: Option<String>,
        cli_callback_port: Option<i32>,
    ) -> Result<String, AuthError> {
        let redirect_after_login = validate_return_to(redirect_after_login)?;
        let client = OidcClient::from_provider_metadata(
            self.metadata().await?,
            ClientId::new(self.config.client_id.clone()),
            Some(ClientSecret::new(self.config.client_secret.clone())),
        )
        .set_redirect_uri(self.redirect_url()?);
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut authorization = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(pkce_challenge);
        if is_google_issuer(&self.config.issuer) && !self.config.allowed_domains.is_empty() {
            let domain_hint = if self.config.allowed_domains.len() == 1 {
                self.config.allowed_domains[0].as_str()
            } else {
                "*"
            };
            authorization = authorization.add_extra_param("hd", domain_hint);
        }
        let (url, state, nonce) = authorization.url();

        sqlx::query(
            r#"INSERT INTO auth_transactions
               (id, provider_id, state_hash, nonce, pkce_verifier,
                redirect_after_login, cli_callback_port, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(Uuid::new_v4())
        .bind(&self.config.provider_id)
        .bind(hash_secret(state.secret()))
        .bind(nonce.secret())
        .bind(pkce_verifier.secret())
        .bind(redirect_after_login)
        .bind(cli_callback_port)
        .bind(Utc::now() + Duration::minutes(TRANSACTION_TTL_MINUTES))
        .execute(&self.db)
        .await?;

        Ok(url.to_string())
    }

    /// Consume and verify a callback, then resolve it to the internal user.
    pub async fn finish(&self, state: &str, code: String) -> Result<OidcAuthentication, AuthError> {
        let transaction = sqlx::query_as::<_, AuthTransaction>(
            r#"DELETE FROM auth_transactions
               WHERE provider_id = $1
                 AND state_hash = $2
                 AND consumed_at IS NULL
                 AND expires_at > now()
               RETURNING nonce, pkce_verifier, redirect_after_login, cli_callback_port"#,
        )
        .bind(&self.config.provider_id)
        .bind(hash_secret(state))
        .fetch_optional(&self.db)
        .await?
        .ok_or(AuthError::InvalidState)?;

        let client = OidcClient::from_provider_metadata(
            self.metadata().await?,
            ClientId::new(self.config.client_id.clone()),
            Some(ClientSecret::new(self.config.client_secret.clone())),
        )
        .set_redirect_uri(self.redirect_url()?);
        let token_response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|error| AuthError::OAuth(format!("OIDC code exchange setup: {error}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(transaction.pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(|error| AuthError::OAuth(format!("OIDC code exchange failed: {error}")))?;
        let id_token = token_response
            .id_token()
            .ok_or_else(|| AuthError::OAuth("OIDC response omitted the ID token".into()))?;
        let nonce = Nonce::new(transaction.nonce);
        let verifier = client.id_token_verifier();
        let identity = match id_token.claims(&verifier, &nonce) {
            Ok(claims) => owned_identity(claims),
            Err(first_error) => {
                // A cached JWKS may be stale after signing-key rotation. Refresh
                // discovery once and verify the same ID token again.
                *self.metadata.write().await = None;
                let refreshed = OidcClient::from_provider_metadata(
                    self.metadata().await?,
                    ClientId::new(self.config.client_id.clone()),
                    Some(ClientSecret::new(self.config.client_secret.clone())),
                )
                .set_redirect_uri(self.redirect_url()?);
                let refreshed_verifier = refreshed.id_token_verifier();
                let claims = id_token.claims(&refreshed_verifier, &nonce).map_err(|retry| {
                    AuthError::OAuth(format!(
                        "OIDC ID token invalid after metadata refresh: {retry}; initial error: {first_error}"
                    ))
                })?;
                owned_identity(claims)
            }
        };
        self.require_google_hosted_domain(&identity)?;
        let user = self
            .resolve_user(
                &identity.issuer,
                &identity.subject,
                identity.email.as_deref(),
                identity.email_verified,
                identity.hosted_domain.as_deref(),
            )
            .await?;

        Ok(OidcAuthentication {
            user,
            redirect_after_login: transaction.redirect_after_login,
            cli_callback_port: transaction.cli_callback_port,
        })
    }

    async fn resolve_user(
        &self,
        issuer: &str,
        subject: &str,
        email: Option<&str>,
        email_verified: bool,
        hosted_domain: Option<&str>,
    ) -> Result<User, AuthError> {
        if let Some((user_id, status, is_bot)) = sqlx::query_as::<_, (Uuid, String, bool)>(
            r#"SELECT users.id, users.status, users.is_bot
               FROM oidc_identities
               JOIN users ON users.id = oidc_identities.user_id
               WHERE oidc_identities.issuer = $1
                 AND oidc_identities.subject = $2"#,
        )
        .bind(issuer)
        .bind(subject)
        .fetch_optional(&self.db)
        .await?
        {
            if status != "active" || is_bot {
                return Err(AuthError::Forbidden);
            }
            sqlx::query(
                "UPDATE oidc_identities SET last_login_at = now() WHERE issuer = $1 AND subject = $2",
            )
            .bind(issuer)
            .bind(subject)
            .execute(&self.db)
            .await?;
            return sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&self.db)
                .await
                .map_err(AuthError::Database);
        }

        let email = email
            .filter(|_| email_verified)
            .ok_or_else(|| AuthError::Forbidden)?;
        self.require_first_login_domain(issuer, email, hosted_domain)?;

        let mut tx = self.db.begin().await?;
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
            Some(user) if self.config.auto_link_verified_email => user,
            Some(_) => {
                return Err(AuthError::Conflict("account_link_required".to_string()));
            }
            None if self.config.auto_provision => {
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
                .bind(UserRole::User)
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
        .bind(&self.config.provider_id)
        .bind(issuer)
        .bind(subject)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(user)
    }

    fn require_google_hosted_domain(&self, identity: &OidcIdentity) -> Result<(), AuthError> {
        if !is_google_issuer(&identity.issuer) || self.config.allowed_domains.is_empty() {
            return Ok(());
        }
        if value_allowed(
            identity.hosted_domain.as_deref(),
            &self.config.allowed_domains,
        ) {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }

    fn require_first_login_domain(
        &self,
        issuer: &str,
        email: &str,
        hosted_domain: Option<&str>,
    ) -> Result<(), AuthError> {
        let allowed = if is_google_issuer(issuer) {
            value_allowed(hosted_domain, &self.config.allowed_domains)
        } else {
            domain_allowed(email, &self.config.allowed_domains)
        };
        allowed.then_some(()).ok_or(AuthError::Forbidden)
    }
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn domain_allowed(email: &str, allowed_domains: &[String]) -> bool {
    if allowed_domains.is_empty() {
        return true;
    }
    email.rsplit_once('@').is_some_and(|(_, domain)| {
        allowed_domains
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(domain))
    })
}

fn value_allowed(value: Option<&str>, allowed_domains: &[String]) -> bool {
    allowed_domains.is_empty()
        || value.is_some_and(|value| {
            allowed_domains
                .iter()
                .any(|allowed| allowed.trim().eq_ignore_ascii_case(value))
        })
}

fn is_google_issuer(issuer: &str) -> bool {
    issuer.trim_end_matches('/') == GOOGLE_ISSUER || issuer == "accounts.google.com"
}

fn owned_identity(claims: &IdTokenClaims<OidcAdditionalClaims, CoreGenderClaim>) -> OidcIdentity {
    OidcIdentity {
        issuer: claims.issuer().as_str().to_string(),
        subject: claims.subject().as_str().to_string(),
        email: claims.email().map(|email| normalize_email(email.as_str())),
        email_verified: claims.email_verified() == Some(true),
        hosted_domain: claims.additional_claims().hosted_domain.clone(),
    }
}

fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

fn validate_return_to(return_to: Option<String>) -> Result<Option<String>, AuthError> {
    match return_to {
        Some(path) if !path.starts_with('/') || path.starts_with("//") => Err(
            AuthError::BadRequest("return_to must be an internal relative path".into()),
        ),
        value => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_rejects_external_and_scheme_relative_urls() {
        assert!(validate_return_to(Some("https://evil.example".into())).is_err());
        assert!(validate_return_to(Some("//evil.example".into())).is_err());
        assert_eq!(
            validate_return_to(Some("/settings/security".into())).unwrap(),
            Some("/settings/security".into())
        );
    }

    #[test]
    fn domain_allowlist_is_exact_and_optional() {
        assert!(domain_allowed("alice@anything.example", &[]));
        assert!(domain_allowed("alice@EXAMPLE.com", &["example.com".into()]));
        assert!(!domain_allowed(
            "alice@sub.example.com",
            &["example.com".into()]
        ));
    }

    #[test]
    fn google_workspace_allowlist_requires_the_signed_hosted_domain() {
        let allowed = vec!["example.com".into()];

        assert!(value_allowed(Some("EXAMPLE.com"), &allowed));
        assert!(!value_allowed(Some("sub.example.com"), &allowed));
        assert!(!value_allowed(None, &allowed));
        assert!(value_allowed(None, &[]));
        assert!(is_google_issuer("https://accounts.google.com/"));
    }
}

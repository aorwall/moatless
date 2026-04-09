//! OAuth provider trait.

use crate::auth::error::AuthError;
use crate::auth::models::{OAuthTokens, ProviderUserInfo};
use async_trait::async_trait;

/// Generic OAuth provider interface.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Generate the OAuth authorization URL.
    fn generate_auth_url(&self, state: &str) -> Option<String>;

    /// Exchange authorization code for tokens.
    async fn exchange_code_for_tokens(&self, code: &str) -> Result<OAuthTokens, AuthError>;

    /// Get user info using the provided tokens.
    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError>;

    /// Check whether the authenticated user belongs to the given organization.
    /// Returns `true` by default (no restriction) for providers that don't support org checks.
    async fn check_org_membership(
        &self,
        _access_token: &str,
        _org: &str,
    ) -> Result<bool, AuthError> {
        Ok(true)
    }

    /// Optional: extract user group from tokens (e.g., Auth0 JWT claims).
    async fn extract_user_group(&self, _tokens: &OAuthTokens) -> Option<String> {
        None
    }

    /// Optional: generate logout URL (e.g., Auth0 requires server-side logout).
    fn generate_logout_url(&self, _return_to: &str) -> Option<String> {
        None
    }
}

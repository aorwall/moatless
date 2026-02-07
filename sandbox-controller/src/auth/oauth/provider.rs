//! OAuth provider trait.

use async_trait::async_trait;
use crate::auth::error::AuthError;
use crate::auth::types::{OAuthTokens, ProviderUserInfo};

/// Generic OAuth provider interface.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Unique provider ID (e.g., "github", "auth0").
    fn provider_id(&self) -> &str;

    /// Generate the OAuth authorization URL.
    fn generate_auth_url(&self, state: &str) -> Option<String>;

    /// Exchange authorization code for tokens.
    async fn exchange_code_for_tokens(&self, code: &str) -> Result<OAuthTokens, AuthError>;

    /// Get user info using the provided tokens.
    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError>;

    /// Optional: extract user group from tokens (e.g., Auth0 JWT claims).
    async fn extract_user_group(&self, _tokens: &OAuthTokens) -> Option<String> {
        None
    }

    /// Optional: generate logout URL (e.g., Auth0 requires server-side logout).
    fn generate_logout_url(&self, _return_to: &str) -> Option<String> {
        None
    }
}

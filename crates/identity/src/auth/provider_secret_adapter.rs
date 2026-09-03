//! Compatibility adapter from `provider_tokens` to redacted auth secrets.

use crate::auth::models::ProviderAccessToken;
use crate::auth::secrets::RedactedSecret;

pub fn provider_token_secret(token: ProviderAccessToken) -> RedactedSecret {
    RedactedSecret::new(token.access_token)
}

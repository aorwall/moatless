//! Provider token DAO — queries for credential resolution.

use super::models::ProviderAccessToken;
use super::DbPool;

/// Find a provider token by user_id and provider name.
///
/// Queries the `provider_tokens` table which stores user-linked OAuth/API tokens
/// for providers like `github` and `claude`.
pub async fn find_provider_token(
    pool: &DbPool,
    user_id: &str,
    provider: &str,
) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
    sqlx::query_as::<_, ProviderAccessToken>(
        "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = $2",
    )
    .bind(user_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
}

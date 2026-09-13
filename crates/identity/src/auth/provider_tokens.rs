//! The `provider_tokens.auth_method` vocabulary and the sentinels that stand in
//! for a token the column does not hold.

/// `provider_tokens.auth_method` value used on bot-user rows. Switches the
/// canonical GitHub-token resolver from "refresh OAuth token" to "mint a fresh
/// installation token from `GitHubAppRegistry`".
pub const AUTH_METHOD_APP_INSTALLATION: &str = "github_app_installation";

/// Sentinel in `provider_tokens.access_token` for installation-backed rows.
///
/// The real token is minted on demand by the canonical resolver; a path reading
/// the column directly sends this and gets a clear 401, rather than dropping
/// the Authorization header on a blank token.
pub const APP_INSTALLATION_TOKEN_PLACEHOLDER: &str = "<github-app-installation>";

/// `provider_tokens.auth_method` value for a GitHub App *user* access token.
///
/// Named after the App that issued the token, not a way of signing in: a row
/// carrying this is unrelated to how its owner authenticated to Moatless.
pub const AUTH_METHOD_APP_USER: &str = "github_app";

/// `provider_tokens.auth_method` value for rows whose real credential lives in
/// the encrypted `secrets` table.
///
/// The row carries only non-secret metadata (`login`, `token_type`), existing
/// so row-presence consumers — the config endpoint's `connected`, `auth/me`'s
/// `providers[]` — keep working.
pub const AUTH_METHOD_SECRET_REF: &str = "secret_ref";

/// Sentinel written into `provider_tokens.access_token` for `secret_ref` rows.
///
/// Mirrors [`APP_INSTALLATION_TOKEN_PLACEHOLDER`]: a direct reader of the
/// column sees this instead of a usable token, so sending it to a provider
/// yields a clear 401 rather than silent misbehaviour.
pub const SECRET_REF_TOKEN_PLACEHOLDER: &str = "<secret-ref>";

/// `provider_tokens.auth_method` value for a Forgejo OAuth2 access token,
/// refreshed against its own instance near expiry.
///
/// Forgejo OAuth2 grants no scopes, so this credential reaches everything its
/// owner reaches — unlike the scoped personal access token an
/// [`AUTH_METHOD_SECRET_REF`] row points at.
pub const AUTH_METHOD_FORGEJO_OAUTH: &str = "forgejo_oauth";

/// The `provider_tokens.provider` value addressing `host`'s Forgejo credential.
///
/// Forgejo is self-hostable and one person may hold a credential per instance,
/// so the host is part of the provider name rather than a column beside it;
/// free text under `provider_tokens_scope_uq`, so a second instance needs no
/// migration.
pub fn forgejo_provider_key(host: &str) -> String {
    format!("forgejo:{host}")
}

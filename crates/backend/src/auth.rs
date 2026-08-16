//! Authentication — the implementation lives in the `moatless-identity` crate.
//!
//! Permanent facade (see `extractors.rs` for the rationale and the glob caveat).
//! Re-exports the `auth` submodules the backend composes; the census resolves
//! `crate::auth::<submodule>::…` through these explicit re-exports.

pub use moatless_identity::auth::{
    api_key, api_keys, cache, codex_auth_json, codex_scope, credentials, error, github,
    github_scope, identity, jwt, middleware, models, pairing, password, provider_token_dao,
    provider_tokens, routes, sessions, test_user, users, Authentication, AuthenticationPorts,
};

// Test fakes (`StubProviderTokens`, `github_row`) reached from backend unit and
// `db-tests` integration tests. Gated to match the `test_support` module's own
// `#[cfg(any(test, feature = "test-support"))]`; `db-tests` forwards that feature.
#[cfg(any(test, feature = "db-tests"))]
pub use moatless_identity::auth::test_support;

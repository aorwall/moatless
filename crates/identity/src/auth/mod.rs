//! Authentication and user management module.
//!
//! Handles login flows (local email/password, GitHub App, Auth0, OIDC), session
//! management, API key management, and Traefik `ForwardAuth` verification.

pub mod api_key;
pub mod api_keys;
pub mod cache;
pub mod codex_auth_json;
pub(crate) mod codex_blob;
mod connect_return;
pub mod cookie;
pub mod credentials;
pub mod error;
mod forgejo_connect;
pub mod github;
mod github_connect;
pub mod jwt;
pub mod middleware;
pub mod models;
pub mod oauth;
pub mod oidc;
mod oidc_dao;
pub mod pairing;
pub(crate) mod pairing_dao;
pub mod password;
pub mod provider_secret_adapter;
pub(crate) mod provider_token_auth_dao;
pub(crate) mod provider_token_dao;
pub mod provider_tokens;
pub mod routes;
pub mod schemas;
pub mod secrets;
mod sessions;
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod test_user;
pub(crate) mod token_refresh;
pub mod users;

mod authentication;
mod compose;
mod handlers;
mod state;
pub use authentication::{Authentication, AuthenticationPorts};

mod account_dao;
mod account_status;
mod accounts;
mod password_operations;

pub(crate) mod api_keys_dao;
pub(crate) mod users_dao;

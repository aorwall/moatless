//! Authentication and user management module.
//!
//! Handles login flows (local username/password, GitHub, GitHub App, Auth0),
//! session management, API key management, and Traefik ForwardAuth verification.

pub mod api_key;
pub mod api_keys;
pub mod cache;
pub mod cookie;
pub mod credentials;
pub mod error;
pub mod github_app_registry;
pub mod github_app_service;
pub mod identity;
pub mod jwt;
pub mod middleware;
pub mod models;
pub mod oauth;
pub mod password;
pub mod provider_secret_adapter;
pub mod provider_token_dao;
pub mod provider_tokens;
pub mod routes;
pub mod schemas;
pub mod secrets;
pub mod session;
pub mod sessions;
#[cfg(test)]
pub mod test_support;
pub mod test_user;
pub mod token_refresh;
pub mod users;

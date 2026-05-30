//! Authentication and user management module.
//!
//! Handles OAuth login flows (GitHub, GitHub App, Auth0), session management,
//! API key management, and Traefik ForwardAuth verification.

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
pub mod local_user;
pub mod middleware;
pub mod models;
pub mod oauth;
pub mod provider_secret_adapter;
pub mod provider_token_dao;
pub mod provider_tokens;
pub mod routes;
pub mod schemas;
pub mod secrets;
pub mod session;
pub mod sessions;
pub mod test_user;
pub mod token_refresh;
pub mod users;

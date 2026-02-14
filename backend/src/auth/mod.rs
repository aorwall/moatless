//! Authentication and user management module.
//!
//! Handles OAuth login flows (GitHub, Auth0), session management,
//! API key management, and Traefik ForwardAuth verification.

pub mod api_key;
pub mod api_keys;
pub mod cache;
pub mod cookie;
pub mod error;
pub mod models;
pub mod oauth;
pub mod provider_token_dao;
pub mod provider_tokens;
pub mod routes;
pub mod session;
pub mod sessions;
pub mod schemas;
pub mod users;

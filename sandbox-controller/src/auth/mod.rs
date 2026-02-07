//! Authentication and user management module.
//!
//! Handles OAuth login flows (GitHub, Auth0), session management,
//! API key management, and Traefik ForwardAuth verification.

pub mod api_key;
pub mod cookie;
pub mod error;
pub mod oauth;
pub mod routes;
pub mod session;
pub mod types;

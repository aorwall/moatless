//! Local username/password authentication.
//!
//! Active when [`crate::config::AuthProvider::Local`] is configured. Credentials
//! live in the `user_credentials` table, hashed with argon2id.

pub mod bootstrap;
pub mod dao;
pub mod hash;
pub mod models;
pub mod throttle;

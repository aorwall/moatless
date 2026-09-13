//! Local email/password authentication.
//!
//! Active when [`moatless_types::auth::AuthProvider::Local`] is configured.
//! Credentials live in the `user_credentials` table, hashed with argon2id.

pub mod bootstrap;
pub(crate) mod dao;
pub mod hash;
pub mod models;
pub mod throttle;
mod throttle_dao;

//! Secret requests: an agent asks a person for a secret in the thread.
//!
//! The person answers in T3's secret card, drawn by T3 rather than by any page
//! the agent wrote. The value goes straight to `SecretService`; the request and
//! its answer are task events that carry no value.

pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod routes;
pub mod schemas;
pub mod service;

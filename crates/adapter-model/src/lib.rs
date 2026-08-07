// reason: test code may unwrap/expect freely; production code keeps the
// workspace `deny`/`warn`.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Adapter domain types (loop/binding contexts, DB rows, inbound/outbound
//! messages, access-control vocabularies) and `adapter_connections`
//! persistence.
//!
//! Lifted out of `moatless-vibe-backend::adapters` so the binding and adapter
//! layers can name these types without pulling in the whole backend.

pub mod dao;
pub mod footer;
pub mod models;

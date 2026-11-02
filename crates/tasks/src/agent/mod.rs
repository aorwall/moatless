//! Agent catalog module — static list of supported agents + credential
//! availability.

pub use moatless_core::agent_catalog as catalog;
pub mod error;
pub mod handlers;
pub mod prompt;
pub mod routes;
pub mod schemas;
pub mod service;
pub(crate) mod side_chat;

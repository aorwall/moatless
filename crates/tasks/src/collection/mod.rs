//! Record collections: JSON records under keys, owned by an Artifact or by a
//! scope a Task reaches, so the Tasks of one Loop or Workspace keep
//! structured data that outlives each of them.

pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod keys;
pub mod models;
pub mod operations;
pub mod routes;
pub mod service;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod watch;

//! Artifacts: HTML pages an agent publishes from its Task, stored in S3 so they
//! outlive the sandbox, and served back to whoever can read the Task.

mod bootstrap;
pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod operations;
pub mod routes;
pub mod schemas;
pub mod service;

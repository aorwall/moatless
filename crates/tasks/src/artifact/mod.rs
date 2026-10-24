//! Artifacts: HTML pages an agent publishes from its Task, stored in S3 so they
//! outlive the sandbox, and served back to whoever can read the Task.

mod bootstrap;
pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod notice_reply;
pub mod operations;
pub mod page_token;
pub mod record_handlers;
pub mod record_socket;
mod record_watch;
pub mod records;
pub mod routes;
pub mod rules;
pub mod schemas;
pub mod service;

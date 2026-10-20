//! Memory: what agents keep for later Tasks, stored outside the sandbox.
//!
//! A request is always made for a Task, which supplies every scope id, so a
//! caller never names another Workspace's or Loop's memory.

pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod operations;
pub mod preload;
pub mod routes;
pub mod schemas;
pub mod service;

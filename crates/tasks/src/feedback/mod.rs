//! Feedback domain module — captures qualitative comments on tasks, messages,
//! tool calls, and preview failures.

pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod operations;
pub mod routes;
pub mod schemas;

pub mod service;

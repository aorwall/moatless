//! Plugin domain module.

pub mod activation;
pub mod archive;
pub(crate) mod dao;
pub mod error;
mod files;
pub(crate) mod handlers;
pub mod models;
pub mod operations;
pub mod routes;
pub mod schemas;
pub mod sync;

pub(crate) mod activation_access;
pub(crate) mod registry;

pub use operations::Plugins;

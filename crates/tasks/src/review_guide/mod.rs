//! Guided reviews: an agent-written tour of a pull request.
//!
//! A guide is stored once per pull request. A run is a Task started to write one; its agent submits the guide
//! with `moat review-guide submit`, and the guide is held to the change's lines
//! before it is stored.

pub(crate) mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod normalize;
pub mod operations;
pub mod routes;
pub mod schemas;
pub mod service;
#[cfg(test)]
mod service_tests;

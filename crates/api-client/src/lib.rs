//! Hand-written typed HTTP client for the Moatless API.
//!
//! Uses types from `moatless-api-schemas` for request/response serialization.

mod client;
mod error;

pub use client::ApiClient;
pub use error::ApiError;
pub use moatless_api_schemas as types;

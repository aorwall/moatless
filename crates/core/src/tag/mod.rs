//! Tag listing module.

pub mod dao;
pub mod error;
pub mod handlers;
pub mod models;
// The OpenApiRouter assembly needs utoipa; a non-API consumer gets the DAO/types.
#[cfg(feature = "openapi")]
pub mod routes;
pub mod schemas;

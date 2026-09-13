//! The standard JSON error body every API module returns.

use serde::Serialize;

/// Standard JSON error body. All API errors MUST use this shape.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: String,
}

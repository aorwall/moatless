---
paths:
  - "backend/src/*/error.rs"
---

# Error Type Pattern

## Structure
```rust
#[derive(Debug, thiserror::Error)]
pub enum XxxError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
    // Add module-specific variants as needed
}
```

## IntoResponse Implementation
```rust
impl IntoResponse for XxxError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Db(e) => {
                tracing::error!(error = %e, "Database error in xxx handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error in xxx handler");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
        };
        error_response(status, message)
    }
}
```

## Security Rules
- NEVER expose database errors or stack traces to clients
- Log internal details with `tracing::error!()`, return generic "Internal server error"
- All responses use `crate::error::error_response()` for consistent `ErrorBody { error }` shape

**Reference**: `backend/src/task/error.rs`

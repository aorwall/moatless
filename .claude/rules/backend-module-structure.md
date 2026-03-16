---
paths:
  - "backend/src/**/*.rs"
---

# Backend Module Structure

Every domain module in `backend/src/` follows this layout:

```
{module}/
├── mod.rs       — Only `pub mod` declarations + one-line doc comment. No logic.
├── models.rs    — Domain types: #[derive(sqlx::FromRow)] structs, enums.
│                  Re-export shared enums from moatless_api_schemas::enums.
├── dao.rs       — Repository trait (#[async_trait] + mockall) + Pg impl.
├── schemas.rs   — Re-export DTOs: `pub use moatless_api_schemas::{module}::*;`
│                  Exception: sandbox/schemas.rs defines its own types.
├── handlers.rs  — HTTP handlers with utoipa::path annotations.
├── routes.rs    — OpenApiRouter with split_for_parts().
└── error.rs     — thiserror enum implementing IntoResponse.
```

## Key Rules

**mod.rs** — only module declarations:
```rust
//! Task domain module.
pub mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod schemas;
```

**Cross-module access**: import types via `crate::module::models::Type`. Never reach into another module's `dao` directly from a handler — go through `AppState` trait objects.

**New module checklist**:
1. Create all files above
2. Add `Arc<dyn XxxRepository>` to `AppState` in `state.rs`
3. Wire `PgXxxRepository` in `AppState::new()`
4. Add router in `startup.rs` route merging
5. Register schemas in OpenAPI doc

**Reference implementation**: `backend/src/task/` module.

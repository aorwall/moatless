---
paths:
  - "backend/src/*/handlers.rs"
---

# Handler Conventions

## Signature Pattern
```rust
#[utoipa::path(
    post,
    path = "/api/v1/things",
    tag = "things",
    request_body = CreateThingRequest,
    responses(
        (status = 200, description = "Created", body = CreateThingResponse),
        (status = 404, description = "Not found", body = crate::error::ErrorBody)
    )
)]
pub async fn create_thing(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateThingRequest>,
) -> Result<Json<CreateThingResponse>, ThingError> {
    // ...
}
```

## Extractors
- `State(state): State<AppState>` — always first
- `auth: AuthUser` — required auth. Use `MaybeAuthUser` for optional
- `Path(id): Path<String>` — path parameters. Axum 0.8 uses `{param}` NOT `:param`
- `Query(params): Query<ListParams>` — query string
- `Json(req): Json<T>` — request body (last extractor)

## Access Patterns
- Repositories via `state.tasks`, `state.messages` etc. (trait objects). Never construct DAO directly.
- User context: `auth.user_id`, `auth.scope_context()` for access-controlled queries.

## Error Mapping
- Use `?` with `From` impls for automatic conversion
- Map `Option::None`: `.ok_or_else(|| ThingError::NotFound(id.clone()))?`
- Map specific errors: `.map_err(|e| ThingError::Internal(format!("context: {e}")))?`

## Responsibilities
Handlers **coordinate** — call DAO methods, map errors, return response. Move complex business logic to separate service functions in the module.

**Reference**: `backend/src/task/handlers.rs`

---
paths:
  - "backend/src/*/dao.rs"
---

# DAO Pattern

## Trait Definition
```rust
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait XxxRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<Xxx>, sqlx::Error>;
    async fn list(&self, filter: &XxxFilter) -> Result<(Vec<Xxx>, i64), sqlx::Error>;
    async fn insert(&self, row: &Xxx) -> Result<(), sqlx::Error>;
    async fn update(&self, id: &str, patch: &XxxPatch) -> Result<(), sqlx::Error>;
    async fn delete(&self, id: &str) -> Result<(), sqlx::Error>;
}
```

## Implementation
```rust
pub struct PgXxxRepository(pub DbPool);

#[async_trait]
impl XxxRepository for PgXxxRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<Xxx>, sqlx::Error> {
        sqlx::query_as::<_, Xxx>(
            r#"SELECT col1, col2, col3 FROM xxx WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }
}
```

## SQL Conventions
- Explicit column lists — never `SELECT *`
- Positional params: `$1, $2, ...`
- `fetch_optional` for single row, `fetch_all` for lists, `fetch_one` only when guaranteed
- Pagination: `COUNT(*) OVER()` window function for count + rows in one query
- Return `Result<T, sqlx::Error>` — handler maps to module error

## Supporting Types
- **Filter**: `#[derive(Default)]` struct with `Option<T>` fields for optional filters
- **Patch**: `{Name}Patch` struct with `Option<T>` fields for partial updates

**Reference**: `backend/src/task/dao.rs`

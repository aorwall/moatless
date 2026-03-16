---
paths:
  - "**/*.rs"
---

# Rust Code Conventions

## Imports
Order: `std` → external crates → `crate::` → `super::`. Group with blank lines between sections.

## Error Handling
- Never `unwrap()` in production code. Use `?` or `expect("descriptive reason why this can't fail")`.
- Define typed errors with `thiserror`. Never `Box<dyn Error>` in library code (acceptable only in `main()`).
- Propagate errors with `?`. Convert with `.map_err(...)` or `From` impls.
- `let _ = expr` only for truly fire-and-forget results. For fallible operations, log failures:
  ```rust
  if let Err(e) = fallible_op().await {
      tracing::warn!(error = %e, "operation failed");
  }
  ```

## Async & Concurrency
- `Arc` for shared state across async tasks. Never `Rc` in async code.
- `CancellationToken` from `tokio_util` for graceful shutdown — not ad-hoc booleans.
- Never block the async runtime (no `std::thread::sleep`, `std::fs`, or `Mutex::lock` in async).
- Use `tokio::sync::RwLock`/`Mutex` in async contexts, not `std::sync`.

## Logging
- Always `tracing` macros (`info!`, `warn!`, `error!`, `debug!`). Never `println!`/`eprintln!`.
- Structured fields: `tracing::error!(error = %e, task_id = %id, "message")`.
- Log internal errors before returning opaque HTTP errors to clients.

## Naming
- Files: `snake_case.rs`
- Types/traits: `PascalCase`
- Functions/methods: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Enum variants: `PascalCase`

## Lints & Attributes
- No `#[allow(...)]` without a `// reason: ...` comment.
- Impl `Default` when a struct has `pub fn new() -> Self` with no arguments (clippy `new_without_default`).
- Prefer `pub(crate)` over `pub` for items not used outside the crate.

## Verification
After changes: `cargo clippy -p <crate> -- -D warnings && cargo test -p <crate>`

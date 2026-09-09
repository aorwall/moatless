# crates/core

## What each module owns

Each module's `//!` doc states what it owns. Three rules bind them:

- `file_storage`'s store is private. A peer crate takes `Files`, and an
  operation over an upload is a method on it rather than a port over the rows.
- No other crate declares a migration directory. `db::run_migrations` runs them
  for the whole workspace.
- Keep SQL construction out of `moatless-types`. `db::access_control` is where
  the shared access policy becomes a predicate; `access_filter`'s own doc states
  how a caller binds it.

# crates/core

## What each module owns

Each module's `//!` doc states what it owns. Three rules bind them:

- Do not add a read port over `file_storage`'s `FileDao`. A peer crate names it
  directly.
- No other crate declares a migration directory. `db::run_migrations` runs them
  for the whole workspace.
- Keep SQL construction out of `moatless-types`. `db::access_control` is where
  the shared access policy becomes a predicate; `access_filter`'s own doc states
  how a caller binds it.

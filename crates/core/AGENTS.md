# crates/core

## What each module owns

- `http_auth` holds the axum request extractors (`AuthUser`, `MaybeAuthUser`,
  `OptionalScope`) and the two ports they are built over, `UserTokenVerifier`
  and `TaskAccess`. `moatless-identity` and `moatless-tasks` implement the two
  ports, and neither crate is a dependency here.
- `file_storage` owns the `file_metadata` table and its `FileDao` store. A peer
  crate names `FileDao` directly; do not add a read port over it.
- `db` opens the pool and runs the migrations for the whole workspace
  (`run_migrations`). No other crate declares a migration directory.
  `db::access_control` translates the shared access policy into SQL predicates;
  keep SQL construction out of `moatless-types`.

Bind the `db::access_control` predicate parameters from one `ScopeContext`:
`is_admin` first, then `user_id`, starting at the index passed to the helper.
The free functions build SQL only; mixing callers or reversing bindings can
change which rows the query admits.

# crates/core

## What each module owns

- `http_auth` holds the axum request extractors (`AuthUser`, `MaybeAuthUser`,
  `OptionalScope`) and the two ports they are built over, `UserTokenVerifier`
  and `TaskAccess`. Every band that mounts a router needs this plumbing, so it
  sits below all of them. `moatless-identity` and `moatless-tasks` implement the
  two ports, and neither crate is a dependency here.
- `file_storage` owns the `file_metadata` table and its `FileDao` store. Every
  consumer is a band above, so the edge is already downward. A read port over it
  would narrow nothing, and a peer crate names `FileDao` directly.
- `db` opens the pool and runs the migrations for the whole workspace
  (`run_migrations`). No other crate declares a migration directory.

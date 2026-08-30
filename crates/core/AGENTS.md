# crates/core

The lowest backend band: configuration, the database pool and migrations, the
error and event vocabularies, file storage, HTTP auth plumbing, leader
election, metrics, and secret keys.

## What lives here

- `http_auth` holds the axum request extractors (`AuthUser`, `MaybeAuthUser`,
  `OptionalScope`) and the two ports they are built over, `UserTokenVerifier`
  and `TaskAccess`. They are plumbing every band that mounts a router needs, so
  they sit below all of them. `moatless-identity` and `moatless-tasks`
  implement the two ports; neither is a dependency here.
- `file_storage` owns the `file_metadata` table and its `FileDao` store. Every
  consumer is a band above, so the edge is already downward and a read port over
  it would narrow nothing — a peer crate names `FileDao` directly.
- `db` opens the pool and runs the migrations for the whole workspace
  (`run_migrations`). No other crate declares a migration directory.

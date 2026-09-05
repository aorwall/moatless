# crates/identity

The identity domain: authentication, team membership, the adapter-app
registry, secret storage, and first-run setup.

## Ownership

- **Users, tokens, teams, secrets, adapter apps** — the tables behind
  `auth/`, `team/`, `secret/` and `adapter_apps/`, and the `*Dao` stores over
  them. Those stores are private to this crate; a peer band names the port
  instead. `crates/identity/src/ports.rs` implements every one of them:
  `UserDisplay`, `UserResolver`, `CommitIdentity`, `ProviderIdentityLink`,
  `HarnessCredentials`, `AppCredentials`, `CodexScopeReader`, `SecretResolver`,
  `SecretCatalog`, and `AdapterAppReader`.
- Two stores are named across the boundary on purpose, both by
  `moatless-workspaces`:
  - `ProviderTokenDao` — the workspace credential service owns the
    provider-token lifecycle. It writes the rows this band reads, so it names
    the store it drives rather than a read port beside it.
  - `SecretAuditDao` — one method, and every consumer appends one audit row and
    reads none. A port over it would have the same shape and the same single
    caller pattern.
- The request extractors are axum plumbing over two ports, not identity. They
  live in `moatless_core::http_auth`. This crate implements the caller side of
  `UserTokenVerifier` and nothing more.
- This crate names no provider. An Adapter resolves its stored credentials
  through `AdapterAppReader`, declared in `moatless-adapter-model` beside the
  Adapter vocabulary. Adding a provider must not edit this crate.

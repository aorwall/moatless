# crates/identity

## What this crate owns

This crate owns users, tokens, teams, secrets and adapter apps. It holds the
tables behind `auth/`, `team/`, `secret/` and `adapter_apps/`, and the `*Dao`
stores over them. Those stores are private to this crate, and a peer band names
a port instead. `crates/identity/src/ports.rs` implements every one of them:
`UserDisplay`, `UserResolver`, `CommitIdentity`, `ProviderIdentityLink`,
`HarnessCredentials`, `AppCredentials`, `CodexScopeReader`, `SecretResolver`,
`SecretCatalog`, and `AdapterAppReader`.

`moatless-workspaces` names two stores across the boundary on purpose:

- `ProviderTokenDao`. The workspace credential service owns the provider-token
  lifecycle. It writes the rows this band reads, so it names the store it drives
  rather than a read port beside it.
- `SecretAuditDao`. It has one method. Every consumer appends one audit row and
  reads none, so a port over it would have the same shape and the same single
  caller.

The request extractors are axum plumbing over two ports, and they are not
identity. They live in `moatless_core::http_auth`. This crate implements the
caller side of `UserTokenVerifier` and nothing more.

This crate names no provider. An Adapter resolves its stored credentials
through `AdapterAppReader`, declared in `moatless-adapter-model` beside the
Adapter vocabulary. Adding a provider must not edit this crate.

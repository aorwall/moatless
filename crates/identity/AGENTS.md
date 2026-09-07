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

Adapter installation belongs to the provider crate. An Adapter resolves its stored credentials
through `AdapterAppReader`, declared in `moatless-adapter-model` beside the
Adapter vocabulary. Adding an Adapter must not edit this crate. User login and
git-host credentials remain identity responsibilities.

## GitHub: signing in and having access are separate

`config.auth.provider` decides how somebody signs in.
`provider_tokens.auth_method` decides how their git operations get a token. A
deployment that signs people in with OIDC still hands them GitHub access, by
either of two writers into the same row:

- `auth/github/user_credential.rs` holds the App user token. GitHub App *login*
  (`auth/session.rs`) and *connect* (`auth/github_connect.rs`) both go through
  its one `persist`, because the row they leave behind has to match.
- `git_host_access.rs` holds a Personal Access Token, stored as a secret with
  the row carrying `secret_ref` and a placeholder. Setting one stashes whatever
  App credential it displaced, so removing it later restores a live one. Stash
  and restore the whole `StashedGithubCredential`: an App user token restored
  without its refresh token and expiry leaves a row nothing can renew.

A GitHub App has one registered callback URL and every client shares it, so a
flow started from the T3 client comes back on the frontend's host. That is why
the state cookie in `auth/cookie.rs` carries `config.auth.cookie_domain` too:
host-only, the callback host reads no `oauth_state` and the CSRF check rejects
the flow it was meant to protect. A deployment spanning two hosts must set
`COOKIE_DOMAIN` to a parent of both.

Two asymmetries are deliberate. `github_required_org` gates login only: it
decides who may sign in, and a connect issues no session, because the person is
already signed in by the configured provider and this is their own credential
for their own git operations. Gating it here would be a second, different policy
wearing the same setting's name, and enforcing org membership on *access* wants
its own setting. A connect also never revokes and never re-issues a session — it
writes the credential row and stops. Anything that changes who the caller is
belongs in the login path.

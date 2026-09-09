# crates/identity

## What this crate owns

Users, tokens, teams, secrets and adapter apps: the tables behind `auth/`,
`team/`, `secret/` and `adapter_apps/`, and the `*Dao` stores over them. Those
stores are private to this crate, and a peer band names a port instead.
`crates/identity/src/ports.rs` implements every one: `UserDisplay`,
`UserResolver`, `CommitIdentity`, `ProviderIdentityLink`, `HarnessCredentials`,
`AppCredentials`, `CodexScopeReader`, `SecretResolver`, `SecretCatalog`, and
`AdapterAppReader`.

`credentials/` owns the provider-token credential. Build exactly one
`CredentialService` per process through `compose::credentials`: the
per-`(user, key)` lock that serializes a concurrent store lives inside it, so a
second instance over the same pool serializes nothing. `secret/audit.rs` owns
every append to `secret_audit_events`, through `compose::secret_audit`.

Adapter installation belongs to the provider crate: an Adapter resolves its
stored credentials through `AdapterAppReader`, declared in
`moatless-adapter-model`. Adding an Adapter must not edit this crate. User login
and git-host credentials remain identity responsibilities.

## Auth-session privacy

Auth-session persistence and policy live in private `auth/sessions/` modules.
`Authentication::build` constructs the session DAO and shares it between routes
and middleware. Keep its DAO, rows and helpers private, including under
`test-support`. Run its PostgreSQL tests with
`cargo test -p moatless-identity --features db-tests`.

## GitHub: signing in and having access are separate

`config.auth.provider` decides how somebody signs in.
`provider_tokens.auth_method` decides how their git operations get a token, so a
deployment that signs people in with OIDC still hands them GitHub access. Two
writers reach the same row:

- `auth/github/user_credential.rs` holds the App user token. GitHub App *login*
  (`auth/sessions/service.rs`) and *connect* (`auth/github_connect.rs`) both go through
  its one `persist`, because the row they leave behind has to match.
- `git_host_access.rs` holds a Personal Access Token, stored as a secret with
  the row carrying `secret_ref` and a placeholder. Setting one stashes whatever
  App credential it displaced, so removing it later restores a live one. Stash
  and restore the whole `StashedGithubCredential`: an App user token restored
  without its refresh token and expiry leaves a row nothing can renew.

A GitHub App has one registered callback URL and every client shares it, so a
flow started from the T3 client comes back on the frontend's host. The state
cookie in `auth/cookie.rs` therefore carries `config.auth.cookie_domain`;
host-only, the callback host reads no `oauth_state` and the CSRF check rejects
the flow. A deployment spanning two hosts must set `COOKIE_DOMAIN` to a parent
of both.

Do not gate connect on `github_required_org` — it gates login only — and do not
let a connect revoke or re-issue a session. A connect writes the credential row
and stops. Anything that changes who the caller is belongs in the login path.

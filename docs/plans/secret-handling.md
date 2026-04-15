# Plan: Secrets Feature for Global, Team, and User Secrets

## Current State

Status as of 2026-04-15:

- Phase 2 has an initial backend implementation.
- `scope=global`, `scope=team`, and `scope=user` are implemented in the `secrets` table and backend feature.
- `scope=repository` is intentionally deferred. Sandbox creation is still gated by existing task/repository access checks, and secret resolution checks whether the task creator/team can access the requested global/team/user secret scope.
- Normal secret read APIs return metadata only. Secret writes are effectively write-only.
- Inline secret values are encrypted before storage with local envelope encryption backed by `SECRETS_ENCRYPTION_KEY`. This keeps the database ciphertext-only and leaves a clean seam for AWS KMS.
- `GetCredentials` no longer reads provider fallback credentials from deployment env vars. It resolves scoped secrets first, then compatibility-reads `provider_tokens`.
- `ProvisionPlan` no longer carries raw `git_provider_token`, `claude_oauth_token`, or `anthropic_api_key`.
- Docker and K8s sandbox builders no longer inject those raw provider/agent credentials into container or pod specs.
- Repository `env_vars` remain non-secret config and now reject reserved or likely-secret key names.
- Server-targeted secret delivery is implemented for K8s and Docker. K8s creates a per-task Secret and injects explicit `secretKeyRef` env vars into matching server containers. Docker writes per-server `0600` env files under the shared task data dir and generated server startup commands source those files.

Validation run:

- `cargo clippy -p moatless-vibe-backend -- -D warnings`
- `cargo test -p moatless-vibe-backend`

## Problem Statement

Secrets are currently spread across environment variables, deployment manifests, database rows, and sandbox provisioning payloads. That creates several risks:

1. Backend modules read secrets directly from process env via the central config.
2. Sandbox provisioning can carry raw secret values in `ProvisionPlan`.
3. Docker and K8s sandbox delivery differ: K8s mostly uses `GetCredentials`, while Docker still injects some secrets into container env.
4. Repository `env_vars` can carry values that may be secret, but they are modeled like plain configuration.
5. User and team provider tokens live in `provider_tokens`, but credential resolution is not represented as a single secret-handling subsystem.
6. Helm has both explicit `secretKeyRef` entries and a broad `envFrom` on the backend deployment.
7. AWS should use IAM identity for AWS services and an external secret manager for app secrets, not static AWS keys in app config.

## Goals

- Make every secret source explicit and module-owned.
- Keep non-secret configuration separate from secret values.
- Support global, team, and user-scoped secrets with one feature and one resolution model.
- Avoid passing secret values through API payloads or long-lived pod specs where possible.
- Work in local Docker, Kubernetes, and AWS/EKS.
- Keep sandbox pods from needing broad secret-manager permissions.
- Make logs and `Debug` output safe by default.

## Non-Goals

- Replacing OAuth or provider connection UX in this plan.
- Exposing secret values in read APIs.
- Giving sandbox pods direct AWS Secrets Manager or KMS access by default.
- Supporting arbitrary secret interpolation in `.moatless/config.json`.

## Secret Classes

| Class | Examples | Owner | Storage | Delivery |
| --- | --- | --- | --- | --- |
| Global secrets | Global provider fallbacks, app-wide tokens, OAuth client secrets after migration | Secret feature + platform/admin | `secrets` table or external refs | Backend secret resolver |
| AWS resource identity | S3 access | Platform/admin | IAM role, not app secret | EKS IRSA or Pod Identity |
| User secrets | GitHub OAuth token, GitHub App user token, Claude token, Codex auth, OpenAI key | Secret feature + Auth integration | `secrets` table, with compatibility reads from `provider_tokens` | Backend resolves for allowed user/task |
| Team secrets | Shared GitHub/GitLab/Claude/Codex/OpenAI credentials | Secret feature + Team access control | `secrets` table, with compatibility reads from `provider_tokens` | Backend resolves for team-owned tasks |
| Repository secrets | `STRIPE_SECRET_KEY`, private package token, app-specific env | Deferred | Not implemented in current pass | Repository access remains owned by task/repository flow |
| Sandbox auth tokens | `USER_AUTH_TOKEN` signed by `TASK_TOKEN_SECRET` | Backend sandbox module | Generated token, not user-supplied | Sandbox env only |
| Runtime secret files | Claude `.credentials`, Codex `auth.json` | Sandbox runtime | Workspace/home filesystem with `0600` perms | Written during sandbox init |

## Target Architecture

```
Platform bootstrap config
  -> only values needed before the app can reach its secret store

Secrets feature
  -> stores and resolves global/team/user scoped secrets
  -> replaces ad hoc env-var credential lookups
  -> integrates with provider_tokens during migration
  -> exposes metadata-only management APIs

Backend sandbox credential service
  -> GetCredentials(task_id, authenticated sandbox token)
  -> sandbox init applies agent credentials and writes runtime files

Future server-targeted delivery
  -> per-task Kubernetes Secret or Docker env file for server-targeted scoped secrets
  -> explicit env/valueFrom in target containers when implemented
```

The backend Secrets feature is the trust boundary for storing and resolving secrets. Sandbox pods should authenticate to the backend and receive only the secrets needed for their task.

## New Secrets Feature

Create a first-class backend feature at `backend/src/secret/` that owns secret storage, validation, access checks, redaction, and resolution for all application-managed secrets.

This feature is secure only if the database stores ciphertext, not plaintext. The `secrets` table must be treated as a scoped metadata and ciphertext store.

### Security Requirements

Mandatory requirements:

- Do not add a plaintext secret value column.
- Store inline secret values only as encrypted ciphertext.
- Use envelope encryption with AWS KMS or an equivalent external key manager.
- Keep KMS keys and bootstrap credentials outside the database.
- Decrypt only inside `backend/src/secret/service.rs` after access, scope, and target checks.
- Return metadata only from normal read endpoints.
- Make API writes effectively write-only: callers can set, replace, rotate, and delete values, but cannot read values back through standard APIs.
- Do not log secret values, decrypted values, request bodies containing values, or token-bearing URLs.
- Audit create, update, delete, and resolve operations with actor, scope, key, target, and result, but not value.
- Deliver only task-scoped plaintext to sandbox/runtime consumers.
- Keep plaintext lifetime short: decrypt, build the response or K8s Secret, then drop it.

Known residual risk:

- A compromised backend runtime can access secrets while resolving them. This is inherent for any service that must use secrets. The design minimizes where plaintext exists and who can request it.

Canonical scopes:

| Scope | `scope_id` | Access model |
| --- | --- | --- |
| `global` | `NULL` | Admin/platform only |
| `team` | `team_id` | Team admins/members according to team policy |
| `user` | `user_id` | Owning user and admins |

Deferred scope:

| Scope | Status | Access model |
| --- | --- | --- |
| `repository` | Not implemented in current pass | Existing sandbox creation and repository APIs continue to own repository access checks |

Canonical targets:

| Target | Use |
| --- | --- |
| `agent` | Agent subprocess and agent auth files |
| `server:<name>` | One named server container |
| `all_servers` | All server containers |
| `all` | Agent and servers |
| `backend` | Backend-only secret, not delivered to sandbox |

### Data Model

Add a `secrets` table:

| Column | Notes |
| --- | --- |
| `id` | UUID |
| `scope_type` | `global`, `team`, or `user` |
| `scope_id` | Nullable only for `global` |
| `key` | Env-style name, e.g. `STRIPE_SECRET_KEY` |
| `kind` | `env`, `provider_token`, `runtime_file`, or `deployment` |
| `provider` | Optional provider name such as `github`, `claude`, `codex`, `openai` |
| `target` | `agent`, `server:<name>`, `all_servers`, `all`, or `backend` |
| `value_ciphertext` | Encrypted value, nullable only when using external ref |
| `encryption_nonce` | Nonce/IV for the ciphertext, when required by the algorithm |
| `encrypted_data_key` | Envelope-encrypted data key, if using per-secret data keys |
| `external_ref` | AWS Secrets Manager or SSM reference, optional |
| `kms_key_id` | KMS key identifier or alias used for encryption |
| `encryption_version` | Encryption scheme version for future rotation/migration |
| `created_by` | User ID |
| `updated_by` | User ID |
| `created_at` | Timestamp |
| `updated_at` | Timestamp |
| `last_used_at` | Optional timestamp |
| `version` | Monotonic integer for rotation/audit |

Constraints:

- Unique on `(scope_type, scope_id, key, target)`.
- `scope_id IS NULL` only when `scope_type = 'global'`.
- `key` must match `^[A-Z_][A-Z0-9_]*$`.
- Reject reserved keys such as `TASK_ID`, `BACKEND_GRPC_URL`, `USER_AUTH_TOKEN`, `TASK_TOKEN_SECRET`, `SESSION_JWT_SECRET`, `DATABASE_URL`, and `KUBERNETES_SERVICE_HOST` unless `target = 'backend'` and the caller is platform/admin.
- Only one of `value_ciphertext` or `external_ref` should be set.
- `value_ciphertext` requires encryption metadata for the configured scheme.

Bootstrap exception:

- Values required before the backend can reach the database or decrypt secrets, such as `DATABASE_URL` and KMS/secret-store bootstrap config, may still come from deployment config.
- Application credentials such as `GITHUB_PAT`, `GITLAB_PAT`, `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `OPENAI_API_KEY`, and `CODEX_AUTH_JSON` should not be read directly from env by credential resolution. They should be created as `scope=global` secrets.

### API Shape

The Secrets feature owns the canonical API:

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/v1/secrets?scope=user&scopeId={id}` | List secret metadata for a scope |
| `GET` | `/api/v1/secrets/{id}` | Get one secret metadata record |
| `PUT` | `/api/v1/secrets` | Create or replace a secret value by scope/key/target |
| `PATCH` | `/api/v1/secrets/{id}` | Update metadata or replace value |
| `DELETE` | `/api/v1/secrets/{id}` | Delete a secret |
| `POST` | `/api/v1/secrets/validate` | Validate required secret coverage for a scope |
| `POST` | `/api/v1/internal/secrets/resolve` | Internal resolution API for backend/sandbox workflows |

Normal `GET` responses return metadata only: `id`, `scope_type`, `scope_id`, `key`, `kind`, `provider`, `target`, `source`, `version`, `updated_at`, and `last_used_at`. They must not return secret values.

Do not add a general reveal endpoint in the initial implementation. If interactive value reveal is ever required, add a separate audited admin-only endpoint such as `POST /api/v1/secrets/{id}/reveal`, require an explicit product/security review, and keep it out of the normal `GET` path.

`PUT /api/v1/secrets` request shape:

```json
{
  "scope": "user",
  "scopeId": "user_123",
  "key": "OPENAI_API_KEY",
  "kind": "env",
  "target": "agent",
  "value": "sk_live_...",
  "externalRef": null
}
```

### Repository Scope Deferred

Repository-scoped secret handling is deferred in the current implementation.

Current behavior:

- Repository access remains checked by existing task/repository flows before sandbox creation.
- Secret resolution is based on the task creator's accessible `global`, `team`, and `user` secret scopes.
- Repository `env_vars` remain non-secret config and reject reserved or likely-secret keys.
- No `/api/v1/repositories/{id}/secrets` convenience routes are implemented yet.

Possible future convenience routes, if repository-scoped secrets are reintroduced:

| Method | Path | Implementation |
| --- | --- | --- |
| `GET` | `/api/v1/repositories/{id}/secrets` | Delegates to `GET /api/v1/secrets?scope=repository&scopeId={id}` |
| `PUT` | `/api/v1/repositories/{id}/secrets/{key}` | Delegates to `PUT /api/v1/secrets` with `scope=repository` |
| `DELETE` | `/api/v1/repositories/{id}/secrets/{key}` | Resolves secret ID, then delegates delete |
| `POST` | `/api/v1/repositories/{id}/secrets/validate` | Delegates to `POST /api/v1/secrets/validate` |

Repository `env_vars` should remain non-secret only. Secret values should be created through the Secrets feature.

### Files

Add:

| File | Purpose |
| --- | --- |
| `backend/src/secret/mod.rs` | Feature module declarations |
| `backend/src/secret/models.rs` | Secret domain types and scope/target enums |
| `backend/src/secret/schemas.rs` | Request/response DTOs, value redaction |
| `backend/src/secret/dao.rs` | `secrets` persistence |
| `backend/src/secret/service.rs` | Validation, access checks, encrypt/decrypt, resolution |
| `backend/src/secret/handlers.rs` | Canonical HTTP handlers |
| `backend/src/secret/routes.rs` | Axum router + OpenAPI annotations |
| `backend/src/secret/error.rs` | Secret-specific errors |
| `backend/src/secret/audit.rs` | Audit records for create/update/delete/resolve |

### `backend/src/auth`

Integrates user and team provider secrets with the Secrets feature:

- OAuth client secret loading for GitHub/Auth0/GitHub App.
- Session JWT secret loading and validation.
- `provider_tokens` read/write and token refresh during migration.
- Redacted provider access token and refresh token wrappers.
- Provider-specific token refresh remains auth-owned, while storage/resolution moves to `backend/src/secret`.

Add:

| File | Purpose |
| --- | --- |
| `backend/src/auth/secrets.rs` | Auth-owned secret config and redacted wrappers |
| `backend/src/auth/provider_secret_adapter.rs` | Compatibility adapter between `provider_tokens` and the new Secrets feature |

### `backend/src/repository`

Repository no longer owns secret storage. It owns repository metadata and repo config fetching only:

- Required secret names from `.moatless/config.json`.
- Fetching `.moatless/config.json` with provider tokens without logging token-bearing URLs.
- No repository-scoped secret routes are implemented in the current pass.

### `backend/src/sandbox`

Owns task credential resolution and sandbox delivery:

- `TASK_TOKEN_SECRET` loading and validation.
- `GetCredentials` resolution order.
- Future per-task K8s Secret creation for server-targeted scoped secrets.
- Removal of raw secret fields from `ProvisionPlan`.
- Shared Docker/K8s behavior for credentials.

Add:

| File | Purpose |
| --- | --- |
| `backend/src/sandbox/secrets.rs` | Task credential resolver and target filtering |
| `backend/src/sandbox/k8s/secret.rs` | Per-task K8s Secret create/delete helpers |

### `sandbox`

Owns runtime application of secrets after backend resolution:

- Fetch credentials over gRPC.
- Set process env during single-threaded init.
- Write Claude and Codex credential files with `0600` permissions.
- Optionally write server env files for Docker/local mode.
- Never read global deployment secrets directly.

Add or extend:

| File | Purpose |
| --- | --- |
| `sandbox/src/secrets.rs` | Runtime secret key validation and application helpers |
| `sandbox/src/init/credentials.rs` | Continue gRPC fetch and runtime file persistence |

## Scoped Secrets

All app-managed secrets should use the same scoped model.

| Scope | Examples | Notes |
| --- | --- | --- |
| `global` | Global Claude/OpenAI fallback, global Git provider PAT, OAuth app credentials after bootstrap migration | Replaces environment-variable credential fallback |
| `team` | Team GitHub PAT, team Claude token, team OpenAI key | Available only to tasks in that team |
| `user` | User OAuth/PAT/API credentials | Available only to tasks owned by that user |

Deferred:

| Scope | Examples | Notes |
| --- | --- | --- |
| `repository` | App-specific runtime env like `STRIPE_SECRET_KEY` | Deferred until repository-specific UX and sandbox/server delivery semantics are defined |

Global secrets are not a synonym for deployment env vars. They are records in the Secrets feature with admin-only access. The resolver should query `scope=global` instead of calling `env::var` for application credentials.

### Repo Config References

`.moatless/config.json` can declare required names, not values:

```json
{
  "requiredSecrets": ["STRIPE_SECRET_KEY"],
  "servers": [
    {
      "name": "web",
      "secretRefs": ["STRIPE_SECRET_KEY"]
    }
  ]
}
```

This should be validation metadata only. The backend still resolves values from the Secrets feature or external refs.

## Provider Secrets

Provider secrets should be represented by the same Secrets feature with provider metadata and scope:

| Scope | Source | Use |
| --- | --- | --- |
| `user` | `secrets` table, with compatibility reads from `provider_tokens.user_id` during migration | User-owned OAuth/PAT/API credentials |
| `team` | `secrets` table, with compatibility reads from `provider_tokens.team_id` during migration | Shared team credentials |
| `global` | `secrets` table | Admin-configured provider fallback credentials |

Target providers:

- `github` -> `GITHUB_TOKEN`
- `gitlab` -> `GITLAB_TOKEN`
- `gitness` -> `GITNESS_TOKEN`
- `claude` -> `CLAUDE_CODE_OAUTH_TOKEN` unless an Anthropic API key is chosen
- `codex` -> `CODEX_AUTH_JSON`
- `openai` -> `OPENAI_API_KEY`

Provider tokens should move behind a redacted secret abstraction before they are returned to callers. As a follow-up, store the token value in `secrets` and keep `provider_tokens` only for OAuth metadata that is not secret, or migrate the table fully into the Secrets feature.

Remove env-var provider fallback from resolution:

- Replace `GITHUB_PAT` env fallback with `scope=global`, `provider=github`.
- Replace `GITLAB_PAT` env fallback with `scope=global`, `provider=gitlab`.
- Replace `GITNESS_PAT` env fallback with `scope=global`, `provider=gitness`.
- Replace `ANTHROPIC_API_KEY` env fallback with `scope=global`, `provider=claude`, `key=ANTHROPIC_API_KEY`.
- Replace `CLAUDE_CODE_OAUTH_TOKEN` env fallback with `scope=global`, `provider=claude`, `key=CLAUDE_CODE_OAUTH_TOKEN`.
- Replace `OPENAI_API_KEY` env fallback with `scope=global`, `provider=openai`.
- Replace `CODEX_AUTH_JSON` env fallback with `scope=global`, `provider=codex`.

## Credential Resolution Order

For a task, resolve credentials in this order:

1. Global secrets, when explicitly configured.
2. Team-scoped secrets and provider token compatibility reads for the task's `team_id`.
3. User-scoped secrets and provider token compatibility reads for the task creator.
4. Public/no-auth fallback where supported.

Notes:

- Git clone credentials should prefer provider tokens for the repository provider.
- Scoped secrets should not override platform-reserved keys.
- Agent credentials and server credentials must be separated by `target`.
- If both `CLAUDE_CODE_OAUTH_TOKEN` and `ANTHROPIC_API_KEY` are available, choose by agent/provider policy rather than returning both blindly.
- Do not call `env::var` in the credential resolver except for bootstrap-only values outside the Secrets feature.

## Sandbox Delivery

### Agent Credentials

Use the existing gRPC path:

1. Backend signs sandbox `USER_AUTH_TOKEN` with `TASK_TOKEN_SECRET`.
2. Sandbox calls `GetCredentials(task_id)` over gRPC.
3. Backend verifies token and task ownership context.
4. Sandbox applies only agent-targeted credentials during init.
5. Sandbox writes Claude/Codex credential files with `0600` permissions.

### K8s Server Container Secrets

K8s sidecar env vars are fixed at pod creation, so gRPC-only delivery is not enough for server-targeted secrets.

For each task:

1. Backend resolves server-targeted scoped secrets with target `all`, `all_servers`, and `server:<name>`.
2. Backend creates a per-task Kubernetes Secret in the sandbox namespace.
3. `pod_builder` injects explicit `secretKeyRef` env vars into the target server containers.
4. Secret has labels:
   - `moatless.io/task-id`
   - `app.kubernetes.io/managed-by=moatless`
5. Secret is deleted when the sandbox is deleted, or garbage-collected by label.

### Docker Server Secrets

For local Docker:

1. Backend should not put raw repo/user provider secrets in `ProvisionPlan`.
2. Sandbox fetches credentials via gRPC during init.
3. For server-targeted scoped secrets, sandbox writes a `0600` env file under the shared data dir.
4. Docker server startup sources that file after the workspace-ready sentinel.

This keeps local behavior aligned with K8s while avoiding broad Docker env injection.

## Kubernetes and Helm

Changes:

1. Remove broad backend `envFrom` against the whole app secret.
2. Stop using deployment env as the source for provider credentials and agent fallbacks.
3. Keep explicit `secretKeyRef` only for bootstrap values that must exist before the backend can read the Secrets feature, such as `DATABASE_URL` and any KMS/secret-store bootstrap config.
4. Import previous deployment env credentials into the Secrets feature as `scope=global` records.
5. Remove `agentFallbackSecrets.secretName` as a runtime resolution source; use global secrets instead.
6. Keep `extraSecretEnv` only as a temporary migration escape hatch, with a follow-up to remove it.
7. Add per-task K8s Secret helpers in backend for repository server secrets.

## AWS

Use AWS-native identity instead of static AWS keys:

1. Backend S3 access uses the AWS SDK default credential chain.
2. On EKS, configure IRSA or EKS Pod Identity with `serviceAccount.annotations`.
3. Bootstrap deployment values can come from AWS Secrets Manager or SSM through External Secrets Operator into Kubernetes Secrets.
4. App credentials should be created as `scope=global`, `scope=team`, or `scope=user` records in the Secrets feature, not read from env.
5. Optional scoped secrets can use external refs to AWS Secrets Manager/SSM, but backend resolves them and passes only task-scoped values to sandbox/server targets.
6. Sandbox pods should not receive AWS secret-manager read permissions by default.

## Migration Plan

### Phase 1: Inventory and Redaction

- [x] Add secret classification docs and reserved key list.
- [x] Add redacted wrappers for auth/provider/sandbox secret values.
- Add logs that report secret presence only, never values.
- Remove literal secrets from Helm values files and rotate anything previously committed.
- Add a policy check that blocks plaintext secret columns or normal read APIs returning values.

### Phase 2: New Secrets Feature

- [x] Add `backend/src/secret` with models, DAO, service, routes, handlers, schemas, and errors.
- [x] Add the `secrets` table with `scope_type`, `scope_id`, `key`, `kind`, `provider`, `target`, ciphertext, and encryption metadata.
- [x] Implement envelope encryption before enabling write endpoints.
- [x] Implement audit records before enabling resolve endpoints.
- [x] Add canonical `GET`, `PUT`, `PATCH`, `DELETE`, and validation endpoints.
- [x] Return metadata only from normal `GET` endpoints.
- [x] Move resolver reads for app credentials out of `std::env` and into `scope=global` secrets.
- [x] Defer `scope=repository` and keep repository access gating in existing task/repository flows.

### Phase 3: Global Secret Migration

- [x] Inventory env-var credential fallbacks: `GITHUB_PAT`, `GITLAB_PAT`, `GITNESS_PAT`, `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `OPENAI_API_KEY`, and `CODEX_AUTH_JSON`.
- Create equivalent `scope=global` secret records.
- [x] Remove these env-var fallbacks from credential resolution.
- Keep env only for bootstrap values such as `DATABASE_URL` and KMS/secret-store bootstrap config.

### Phase 4: Provider Token Hardening

- [x] Add redacted types to provider token reads.
- [x] Centralize user/team provider resolution.
- Add tests for user vs team precedence and missing provider fallback.
- Plan or implement migration from `provider_tokens` secret values to `secrets` with `scope=user` and `scope=team`.

### Phase 5: Repository-Scoped Secrets Deferred

- Repository-scoped secret management is not part of the current implementation.
- Repository convenience routes are not implemented.
- Add required secret validation for repositories.
- [x] Keep `repository.env_vars` but validate/block reserved and likely-secret keys.

### Phase 6: Sandbox Delivery

- [x] Remove `git_provider_token`, `claude_oauth_token`, and `anthropic_api_key` from `ProvisionPlan`.
- [x] Make Docker and K8s both rely on `GetCredentials` for agent credentials.
- [x] Add K8s per-task Secret injection for server-targeted scoped secrets.
- [x] Add Docker env-file delivery for server-targeted scoped secrets.

### Phase 7: Helm and AWS

- Replace broad `envFrom` with explicit bootstrap-only secret refs.
- Document External Secrets Operator / AWS Secrets Manager setup.
- Document IRSA/EKS Pod Identity for S3 and any optional external secret refs.
- Add deployment validation notes for missing required secrets.

## Testing Plan

- Unit tests for secret key validation and reserved-key rejection.
- Unit tests for redacted `Debug` output.
- Unit tests proving normal read DTOs cannot serialize secret values.
- Unit tests for encrypt/decrypt round trips through the configured envelope encryption abstraction.
- DAO tests for `secrets` metadata-only reads.
- DAO or migration tests proving there is no plaintext value column.
- Authorization tests for global, team, and user scopes.
- Credential resolver tests:
  - global secret wins when explicitly configured
  - scoped secret target filtering
  - team provider token before user provider token
  - user provider token fallback
  - no secret for public repository clone
  - env-var provider fallback is not used
- Audit tests for create, update, delete, and resolve operations.
- K8s pod builder tests for per-server `secretKeyRef` injection once server-targeted delivery is implemented.
- Docker builder/runtime tests to ensure raw secrets are not in `ProvisionPlan`.
- Integration test that creates a scoped secret, starts a sandbox, and confirms the agent/server receives only the targeted key.

## Open Decisions

1. Should scoped secrets be encrypted in Postgres first, or stored as external refs only?
2. Should team provider credentials override user provider credentials for all tasks, or only team-owned repositories/tasks?
3. Should global provider credentials be fallback-only or explicit admin override?
4. Should repository-scoped secrets be reintroduced, or should repository access remain solely a gate for resolving user/team/global secrets?
5. If repository-scoped secrets are reintroduced, how should they behave for forks and untrusted branches?
6. Should server-targeted secrets be available to all server containers by default, or only with explicit `secretRefs`? Resolved for the current implementation: only targets `all`, `all_servers`, or `server:<name>` are delivered; there is no implicit delivery for arbitrary secrets.

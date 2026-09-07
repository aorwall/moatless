# Installation secrets

Use `moat k8s secrets bootstrap --context <context> --namespace <namespace>
--values values.local.yaml --env-file .env.local` from a local terminal. The values
file scopes which keys are needed. Bootstrap reads explicit/discovered env files
or masked prompts; it does not consume unrelated process environment variables.
Use `secrets list` with the same context/namespace to inspect names, not values.

| Key | Required when |
|---|---|
| `SECRETS_ENCRYPTION_KEY` | Always; base64 of 32 random bytes. Preserve it for an existing database. |
| `TASK_TOKEN_SECRET` | Always; task-scoped authentication. |
| `DATABASE_URL` | External Postgres (`postgres.enabled: false`). |
| `REDIS_URL` | External Redis with `redis.enabled: false` and `redis.urlSecretKey: REDIS_URL`. |
| GitHub App client ID/secret | GitHub App login is selected. |
| Auth0 domain/client ID/client secret | Auth0 login is selected. |
| Object-store bucket, region and credentials | Selected object-storage configuration requires them; use workload identity when configured. |
| `SANDBOX_WORKSPACE_REPOSITORY_SECRET` | `kubernetes.sandboxStorage.workspaceSnapshotFormat` includes `restic`. Preserve it: rotating it makes every existing repository unreadable. |

Runtime agent credentials are user/task configuration; bootstrap does not require
a cluster-wide Anthropic API key. Private-image pull credentials are a separate
Secret and only needed for registries that require them.

GitHub App callback: `https://<app-domain>/api/v1/auth/github-app/callback`.
Auth0 callback: `https://<app-domain>/api/v1/auth/auth0/callback`.
Verify these against the selected provider and application public URL.

Keep `.env.local` untracked and secret values out of chat and logs. Bootstrap's
`--dry-run` prints Secret YAML containing credentials; treat that output as secret.
Use `--overwrite` only when intentionally replacing stored keys. Regenerating the
master encryption key does not repair credentials encrypted under the original.

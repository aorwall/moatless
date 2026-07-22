# Secrets Catalog

Keys the chart consumes from `Secret/vibe-secrets`. `moat k8s secrets list --json` reports which of these are currently stored in the cluster Secret (a `{"keys":[...]}` array of names). `moat k8s secrets bootstrap` is what discovers values from `.env`/prompts and reports missing keys.

## Where to put secret values

**Preferred: a `.env.local` file at the repo root.** `moat k8s secrets bootstrap` will discover it, show you which keys it found, and ask before reading each one. The file stays on your disk — it never enters the chat transcript.

**Avoid pasting secrets directly into chat.** Chat history is persisted, rendered, and sometimes synced to remote transcripts; anything you paste there can leak. The bootstrap command does accept masked terminal prompts as a fallback, but those run in the CLI — Claude should not ask you to type a key into the chat window.

If you must share a key over chat (emergency, no terminal access), tell the user:
> "Pasting secrets in chat works but is less secure than `.env.local`. Keys may be retained in logs/transcripts. Proceed anyway?"

After the Secret is bootstrapped, delete or `.gitignore` the `.env.local` file if it contained production credentials you don't want on disk.

## Always required

| Key | Purpose | Where to get it |
|---|---|---|
| `DATABASE_URL` | Postgres connection string | Provider console (Azure Flexible Server, RDS, Supabase, etc.); omit when `postgres.enabled: true` |
| `SECRETS_ENCRYPTION_KEY` | Master key for credential encryption at rest | `openssl rand -base64 32` |
| `TASK_TOKEN_SECRET` | HMAC for task-scoped backend↔sandbox JWTs. Required — every provision fails without it | `openssl rand -hex 32` |

> `ANTHROPIC_API_KEY` is **not** collected at install time. Each user/task supplies its own Claude credentials at runtime — do not seed a cluster-wide key.

## Only when using external Redis (`redis.enabled: false`)

| Key | Purpose |
|---|---|
| `REDIS_URL` | Redis connection for event streams and rate limiting |

By default the chart deploys a bundled first-party Redis (`redis.enabled: true`) and no `REDIS_URL` is needed. Set `redis.enabled: false` in `values.local.yaml` if you want to point at an existing Redis, then provide `REDIS_URL`.

## Recommended

| Key | Purpose |
|---|---|
| `GITHUB_TOKEN` | PAT for cloning private repos into sandboxes |
| `S3_BUCKET`, `S3_REGION` | File storage (if external storage enabled) |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | Only if not using IRSA |

## GitHub SSO (when `auth.provider=github`)

| Key | Notes |
|---|---|
| `GITHUB_CLIENT_ID` | From a new OAuth App at https://github.com/settings/applications/new |
| `GITHUB_CLIENT_SECRET` | Generated when you save the OAuth App |
| `GITHUB_APP_CLIENT_ID` | GitHub App, if using app-style auth |
| `GITHUB_APP_CLIENT_SECRET` | GitHub App |
| `GITHUB_APP_PRIVATE_KEY` | PEM blob for the GitHub App |

OAuth App callback URL: `https://<app-domain>/api/auth/github/callback`

## Auth0 (when `auth.provider=auth0`)

| Key | Notes |
|---|---|
| `AUTH0_DOMAIN` | Tenant domain, e.g. `mycompany.us.auth0.com` |
| `AUTH0_CLIENT_ID` | From "Regular Web Application" in Auth0 dashboard |
| `AUTH0_CLIENT_SECRET` | Same |

Allowed callback URL: `https://<app-domain>/api/auth/auth0/callback`

## `.env` file template

Place in repo root as `.env.local` and `moat k8s secrets bootstrap` will offer to read it:

```ini
DATABASE_URL=postgres://user:pass@host:5432/db
SECRETS_ENCRYPTION_KEY=<base64 of 32 random bytes — openssl rand -base64 32>
TASK_TOKEN_SECRET=<64 hex chars>
GITHUB_CLIENT_ID=Iv1.xxx
GITHUB_CLIENT_SECRET=xxxxxxxx
GITHUB_TOKEN=ghp_xxxxxxxx
# Only when redis.enabled: false in values.local.yaml
# REDIS_URL=redis://redis.example.com:6379/0
```

## Why process env is off-limits

`moat k8s secrets bootstrap` deliberately does not scan `std::env`. Shell environments are shared across projects, export every child process, and often contain unrelated credentials — auto-consuming them risks pushing the wrong secret into the cluster Secret. Read explicitly from `.env` files or interactive prompts.

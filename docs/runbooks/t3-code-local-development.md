# T3 Code against a local Moatless backend

> **Working inside a Moatless sandbox instead?** This runbook is the Docker
> Compose path, for a laptop. Inside a sandbox the two sides run as Scripts in
> one pod, with different auth, a different rebuild loop, and an Origin
> allowlist that rejects `localhost`. The `t3-development` skill
> (`.agents/skills/t3-development/SKILL.md`) documents that workflow; do not
> follow the Compose steps below inside a sandbox.

This runbook starts a local Moatless Docker Compose environment from the current
Rust checkout and runs the T3 Code web app against its UI-RPC surface. The
Compose stack serves no UI of its own, so you sign in through the T3 dev server.

## Architecture

```text
browser :5733
  │  T3 assets plus same-origin /.well-known, /api and /ws
  ▼
T3 Vite proxy
  │  preserves the moatless_session and moatless_csrf cookies
  ▼
Moatless Rust :8080
  ├─ /.well-known/t3/environment   public discovery
  ├─ /api/v1/auth/password/login   local sign-in, sets the session cookie
  ├─ /api/auth/session             authenticated bootstrap
  └─ /ws                           authenticated JSON RPC
       │
       └─ PostgreSQL
```

Browser cookies are scoped to a host, not a port. Use `localhost`
consistently: a session set on `localhost` is not sent to a page opened on
`127.0.0.1`.

## Prerequisites

- Docker with Compose v2
- ports 5432, 5733 and 8080 available
- Node 24.13.x and Corepack for T3 Code
- the `moatless` and T3 Code repositories checked out as sibling directories

## 1. Prepare local Moatless configuration

From the Moatless repository:

```bash
cp .env.example .env.t3-rpc.local
openssl rand -hex 32
openssl rand -hex 32
openssl rand -base64 32
```

Paste the generated values into `.env.t3-rpc.local` as
`TASK_TOKEN_SECRET`, `SESSION_JWT_SECRET`, and
`SECRETS_ENCRYPTION_KEY`, respectively. Add a local admin; the password needs
at least 12 characters:

```dotenv
AUTH_PROVIDER=local
ADMIN_EMAIL=admin@example.com
ADMIN_PASSWORD=change-me-local-only
```

The `.env.*.local` pattern is ignored by Git. The example contains placeholders,
not usable production secrets.

## 2. Start Moatless from the current Rust checkout

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  up -d --build postgres backend
```

The overlay builds `docker/Dockerfile.backend` instead of pulling the published
backend image, and enables:

```dotenv
T3_UI_RPC_ENABLED=true
T3_UI_RPC_ALLOWED_ORIGINS=http://localhost:5733,http://localhost:8080
```

The second allowed origin accounts for dev-proxy implementations that rewrite
`Origin` to their target. Each entry is matched exactly and a valid Moatless
cookie is still required, preventing cross-site WebSocket use. An entry may
instead name a host of the form `*.<domain>`, admitting any host under that
domain on the same scheme — a deployment serving each sandbox at its own
hostname needs that, a local checkout does not.

The published `environmentId` is a fixed name with a digest of the database's
identity appended — `moatless-3f9a1c0e4b7d8a52` — because T3 keys its cached
listing on that id, and a listing built against one database must not be reused
against another. Recreating the compose Postgres volume changes the published
id, and every connected client rebuilds its listing. That is intended.

Check startup:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  logs -f backend
```

Once the backend is up, discovery is public and the session route is not:

```bash
curl -i http://localhost:8080/.well-known/t3/environment
curl -i http://localhost:8080/api/auth/session
```

The first request should return `200`, the second `401`.

## 3. Run T3 Code against the Compose backend

From the sibling T3 Code repository:

```bash
corepack enable
pnpm install --frozen-lockfile
T3CODE_PROXY_TARGET=http://localhost:8080 pnpm dev:web
```

For a persistent checkout-local setting, put this in T3 Code's ignored
`.env.local` instead:

```dotenv
T3CODE_PROXY_TARGET_OVERRIDE=http://localhost:8080
```

## 4. Sign in

Open [http://localhost:5733](http://localhost:5733), then sign in from the
browser's developer console on that page, so the cookies land on the T3 origin:

```js
await fetch("/api/v1/auth/password/login", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ email: "admin@example.com", password: "change-me-local-only" }),
}).then((r) => r.status);
```

A `200` sets `moatless_session` and `moatless_csrf`. Reload the page: it should
connect to the `Moatless` environment and list one sidebar group per Workspace
the signed-in user may read, without a pairing prompt.

## 5. Check the connection

1. `/api/auth/session` answers `200` through the T3 proxy;
2. the `/ws` request is `101 Switching Protocols`;
3. `orchestration.subscribeShell` receives a `Chunk` whose snapshot lists one
   project per readable Workspace, each with a distinct `workspaceRoot`;
4. a page left open keeps exchanging JSON `Ping`/`Pong`.

Browser developer tools can inspect the `/ws` frames. Backend logs include the
authenticated Moatless user id for session, shell, and socket bootstrap. The
`rpc-verification` skill probes individual methods from a script.

## Rebuild and stop

After Rust changes, rebuild only the backend:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  up -d --build backend
```

Stop the environment without deleting the PostgreSQL volume:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  down
```

Do not add `--volumes` unless destroying the local database is intentional.

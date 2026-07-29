# T3 Code against the Moatless Rust backend

This runbook starts a local Moatless Docker Compose environment from the current
Rust checkout, authenticates through the existing Moatless UI, and runs the T3
Code web app against the opt-in Rust compatibility surface.

The target is a **connected T3 environment showing the viewer's projects and
followed conversations**:
discovery, cookie authentication, HTTP bootstrap, WebSocket upgrade, and the
JSON Effect-RPC envelopes end to end, with the shell listing readable Workspaces
and the Moatless Tasks the logged-in user follows. Most non-shell commands are
still refused or exposed as empty bootstrap streams.

## Architecture

```text
browser :5733
  │  T3 assets plus same-origin /.well-known, /api and /ws
  ▼
T3 Vite proxy
  │  preserves the moatless_session cookie
  ▼
Moatless Rust :8080
  ├─ /.well-known/t3/environment   public discovery
  ├─ /api/auth/session             authenticated bootstrap
  ├─ /api/orchestration/shell      authenticated shell snapshot
  └─ /ws                           authenticated JSON RPC
       │
       ├─ PostgreSQL
       └─ Redis

Moatless UI :3000 is used only to establish the normal local login session.
```

Browser cookies are scoped to a host, not a port. A session established at
`http://localhost:3000` is therefore also sent to the T3 page at
`http://localhost:5733`, whose Vite proxy forwards it to port 8080. Use
`localhost` consistently; logging in on `localhost` and opening T3 on
`127.0.0.1` creates two different cookie hosts.

## Prerequisites

- Docker with Compose v2
- ports 3000, 5432, 5733, 6379, and 8080 available
- Node 24.13.x and Corepack for T3 Code
- the `moatless` and `t3code` repositories checked out as sibling directories

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
`SECRETS_ENCRYPTION_KEY`, respectively. Add an explicitly local admin password:

```dotenv
AUTH_PROVIDER=local
ADMIN_USERNAME=admin
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
  up -d --build postgres redis backend frontend
```

The overlay builds `docker/Dockerfile.backend` instead of pulling the published
backend image, starts the published frontend without its template-rendering
entrypoint so the read-only local API proxy remains in place, and enables:

```dotenv
T3_UI_RPC_ENABLED=true
T3_UI_RPC_ENVIRONMENT_ID=moatless-compose
T3_UI_RPC_LABEL=Moatless Compose
T3_UI_RPC_SERVER_VERSION=0.0.28
T3_UI_RPC_ALLOWED_ORIGINS=http://localhost:5733,http://localhost:8080
```

The second allowed origin accounts for dev-proxy implementations that rewrite
`Origin` to their target. The backend still requires an exact match and a valid
Moatless cookie, preventing cross-site WebSocket use.

`T3_UI_RPC_ENVIRONMENT_ID` is not the id the backend publishes. The published
`environmentId` is this value with a digest of the database's identity appended
— `moatless-compose-3f9a1c0e4b7d8a52` — because T3 keys its cached listing on
that id, and a listing built against one database must not be reused against
another. Recreating the compose Postgres volume therefore changes the published
id, and every connected client rebuilds instead of resuming into a database
that no longer contains what it holds. That is intended.

Check startup:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  ps

docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  logs -f backend
```

The backend log should contain `T3 UI RPC compatibility routes registered`.

## 3. Establish the Moatless browser session

Open [http://localhost:3000](http://localhost:3000) and sign in with the local
admin credentials. Leave this tab signed in.

The discovery endpoint is public, but the session, shell, and WebSocket routes
are not:

```bash
curl -i http://localhost:8080/.well-known/t3/environment
curl -i http://localhost:8080/api/auth/session
```

The first request should return `200`; the unauthenticated command-line session
request should return `401`.

## 4. Run T3 Code against the Compose backend

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

Open [http://localhost:5733](http://localhost:5733). The application should
connect to an environment labelled `Moatless Compose` and render one sidebar
group per Workspace the logged-in user may read, each group empty, without a
pairing prompt. An admin session sees every Workspace in the deployment.

## 5. Acceptance checks

The phase is successful when all of the following hold:

1. discovery returns a Linux/x64-or-arm64 Moatless descriptor;
2. `/api/auth/session` is `401` without a cookie and authenticated through the
   browser proxy;
3. the `/ws` request is `101 Switching Protocols`;
4. `server.getConfig` receives a successful `Exit`;
5. `orchestration.subscribeShell` receives a `Chunk` whose snapshot lists one
   project per readable Workspace — distinct `workspaceRoot` values, so two
   Workspaces are two sidebar groups — plus `synchronized`;
6. the client sends `Ack` for the chunk and remains connected;
7. a page left open continues to exchange JSON `Ping`/`Pong`;
8. no Moatless Task, Message, or Workspace is mutated.

Browser developer tools can inspect the `/ws` frames. Backend logs include the
authenticated Moatless user id for session, shell, and socket bootstrap.

## Implemented phase-one surface

### HTTP

| Route | Authentication | Result |
|---|---|---|
| `GET /.well-known/t3/environment` | public | T3 execution-environment descriptor |
| `GET /api/auth/session` | Moatless cookie/API key | authenticated read-only T3 session |
| `GET /api/orchestration/shell` | Moatless cookie/API key | the viewer's projects; no threads yet |
| `GET /ws` | Moatless cookie/API key plus allowed Origin | Effect-compatible WebSocket |

### RPC

| Behavior | Methods |
|---|---|
| handshake/config | `server.probe`, `server.getConfig`, `server.getSettings`, `server.refreshProviders` |
| empty reads | `server.discoverSourceControl`, `preview.list`, `orchestration.replayEvents`, `cloud.getRelayClientStatus` |
| projected reads | `orchestration.getArchivedShellSnapshot` (projects only) |
| initialized streams | `orchestration.subscribeShell`, `subscribeServerConfig`, `subscribeServerLifecycle`, `subscribeAuthAccess` |
| intentionally idle streams | `subscribeVcsStatus`, `subscribeTerminalEvents`, `subscribeTerminalMetadata`, `subscribePreviewEvents`, `subscribeDiscoveredLocalServers` |
| visible failure | every other RPC receives a request-local unsupported defect |

The protocol accepts one envelope or an array per WebSocket message, preserves
decimal request ids as strings, caps message/frame and active-stream counts,
answers Effect keepalives, records stream acknowledgement, and terminates an
interrupted stream. Initial streams stay open.

## Current limitations and next implementation phase

This is a transport kernel, not the complete server:

- the shell lists projects but never a thread;
- no Moatless event source feeds a live RPC stream;
- only one initial chunk is produced per initialized stream, so queued
  multi-chunk delivery, acknowledgement timeout, and slow-consumer policy are
  still pending;
- unsupported calls fail as defects rather than typed method errors;
- the server version is configured manually to match the T3 checkout;
- WebSocket tickets, bearer sessions, remote origins, T3 Connect, terminal,
  preview automation, VCS, files, diffs, and every write remain unavailable.

The next vertical slice should be read-only domain projection:

1. introduce owned Rust DTOs/codecs and golden wire fixtures captured from the
   pinned Effect implementation;
2. project authorized Moatless Workspaces/Repositories into T3 Projects;
3. project authorized Tasks into T3 Thread shells;
4. load one Task's assembled Messages as a Thread snapshot;
5. expose a typed backend event port and translate Task/Message changes into
   sequenced shell/thread events with replay;
6. only then add a narrow command such as `thread.create`, with idempotency and
   Moatless authorization tested independently of the transport.

## Rebuild and stop

After Rust changes, rebuild only the backend:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  up -d --build backend
```

Stop the environment without deleting PostgreSQL or Redis volumes:

```bash
docker compose \
  --env-file .env.t3-rpc.local \
  -f docker-compose/docker-compose.yml \
  -f docker-compose/docker-compose.t3-rpc.yml \
  down
```

Do not add `--volumes` unless destroying the local database is intentional.

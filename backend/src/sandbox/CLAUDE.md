# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

The `sandbox` module is a Rust (Axum) service that manages the full lifecycle of Kubernetes-based sandbox environments. Each sandbox is a multi-container pod identified by a `task_id`, provisioned on demand, and monitored via a K8s pod watcher. The module runs as part of the `moatless-vibe-backend` binary.

## Architecture

### Provider Abstraction

`SandboxProvider` trait (`provider.rs`) defines the provider-agnostic interface for sandbox lifecycle operations: provision, start, stop, restart, cleanup, status, server logs, file search, and `sandbox_client()`. Implementations: `K8sSandboxProvider` (`k8s/provider.rs`) and `DockerSandboxProvider` (`docker/provider.rs`).

### Request Flow

```
HTTP request → routes.rs (Axum OpenApiRouter)
             → handlers.rs (lifecycle: provision, stop, start, restart, cleanup, status, search)
             → file_handlers.rs (file ops: tree, read, write, list, search-by-name)
                 ├→ provider.sandbox_client() → SandboxClient (gRPC direct)
             → SandboxProvider impl (K8s or Docker)
             → k8s/* or docker/* modules
```

### K8s Resources per Sandbox

Provisioning a sandbox creates these resources (all labeled `moatless.io/task-id={task_id}`):

1. **Deployment** — wraps the pod spec, enables stop (scale to 0) / start (scale to 1) without re-provisioning
2. **Pod** (via Deployment) — contains a `sandbox` container + dynamic server containers (`files-server`, `preview-server`, etc.)
3. **Service** (ClusterIP) — exposes sandbox HTTP (8001), gRPC, and server ports
4. **PVC** — optional persistent workspace storage
5. **IngressRoute** (Traefik CRD) — optional, one per server port, pattern: `{task_id}--{port}.{proxy_domain}`

Deployment and Service names are deterministic: `moatless-ws-{sanitized_task_id}` (see `k8s/naming.rs`).

### Pod Structure

Each pod has:
- **`sandbox` container**: main agent environment (image configurable per-repo), ports 8001 (HTTP) + gRPC, communicates back to the backend via gRPC
- **Server containers** (dynamic, fully data-driven from `ServerConfig`): each server specifies its own `docker_image` (falls back to `sandbox_image`), `start_commands`, `env_vars`, port, and capabilities. Container name convention: `{server.name}-server`. Readiness probe: TCP on server port.
- **Volumes**: `workspace-data` (PVC), `shared-data` (emptyDir), `build-cache` (emptyDir), `tmp` (memory-backed emptyDir)

### Status Resolution

`StatusResolver` (`k8s/status_resolver.rs`) resolves sandbox status directly from the K8s API:
1. Check Deployment replicas (0 = Stopped, missing = NotFound)
2. Check actual pod phase and sandbox container readiness
3. Server status: overlay K8s container probe states onto repo-config-defined servers (config-first)

There is no Redis caching layer for status — all reads go directly to the K8s API.

### Pod Watcher

`pod_watcher.rs` is a leader-elected (K8s Lease) background task that watches all sandbox pods and:
- Detects lifecycle transitions (OOM, crash loop, eviction, restart, ready, failed, deleted)
- Publishes events to Redis Streams (sandbox, server, agent events) for SSE and event handler consumption
- Tracks pods by pod_name (not task_id) to handle rolling updates correctly

### Task Reaper

The unified task reaper lives in `task/reaper.rs` (not in this module). It is a leader-elected background task that on each cycle: (1) scales idle sandbox Deployments to 0 after `ttl_minutes`, and (2) auto-closes Open tasks inactive past `inactive_ttl_days` via the same `cleanup` + `update_task(Closed)` + `TaskSubType::Closed` sequence as the manual close handler.

### gRPC Communication

`SandboxClient` (`client.rs`) calls sandbox pods via gRPC: execute-agent, stop-agent, notify-messages, file operations (tree, read, write, list, search-by-name), and search-files. Uses lazy connection with 5s connect timeout, 10s request timeout. The `classify_grpc_error()` function maps gRPC errors to `SandboxError` (Unavailable→NotReady/503, others→Internal/500).

File operation handlers (`file_handlers.rs`) call `provider.sandbox_client()` to get a ready `SandboxClient`, then invoke file methods directly — no provider-level duplication.

`notify.rs` provides fire-and-forget convenience for notifying a sandbox that new messages are available.

### Auto-Provisioning

`k8s/auto_provision.rs` enables `start()` to transparently provision a workspace that was never provisioned by reading task + repository from the DB and resolving credentials (provider_tokens table → env var fallback).

### JWT Auth

`jwt.rs` generates/verifies HS256 JWTs (90-day expiry, issuer `moatless-vibe-backend`) that sandbox pods use to authenticate back to the backend.

## Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `SandboxClient` | `client.rs` | gRPC client wrapper for sandbox pod calls |
| `ClientError` | `client.rs` | Error constructing a `SandboxClient` |
| `classify_grpc_error()` | `client.rs` | Maps `tonic::Status` → `SandboxError` (Unavailable→NotReady, else→Internal) |
| `ServerConfig` | `models.rs` | Data-driven server definition: name, port, docker_image, start_commands, env_vars, default, enable_inspector |
| `ServerSnapshot` | `models.rs` | Cached server state: config fields + runtime status/error. Built via `from_config(&ServerConfig, status)` |
| `ServerCapabilities` | `models.rs` | Derived from `enable_inspector` via `server_capabilities_from_config()` / `server_capabilities_from_snapshot()` |
| `EnvironmentStatus` | `models.rs` | `not_created`, `initializing`, `ready`, `stopped`, `removing`, `error` |
| `PodPhase` | `models.rs` | K8s pod phase + synthetic `Stopped` (replicas=0), `NotFound` |
| `WorkspaceStatusFullResponse` | `schemas.rs` | Frontend response with servers, proxy URLs, capabilities |
| `ProvisionRequest` / `ProvisionPlan` | `models.rs` | Provisioning input (repo config, git URL, credentials, env vars) |
| `SandboxError` | `error.rs` | Unified error type, maps to HTTP status codes |
| `ContainerObservedStatus` | `models.rs` | K8s container state mapped to server status strings |
| `ErrorBody` | `schemas.rs` | Generic error body for utoipa error annotations |

## API Endpoints

All endpoints are under `/api/sandbox/v1/tasks/{task_id}/`:

| Method | Path | Handler file | Description |
|--------|------|-------------|-------------|
| POST | `/provision` | `handlers.rs` | Create sandbox resources |
| GET | `/env-status` | `handlers.rs` | Lightweight environment status |
| GET | `/status` | `handlers.rs` | Full workspace status (env + servers + agent) |
| POST | `/stop` | `handlers.rs` | Scale Deployment to 0 |
| POST | `/start` | `handlers.rs` | Scale to 1 (or auto-provision) |
| POST | `/restart` | `handlers.rs` | Delete pods (Deployment recreates) |
| POST | `/cleanup` | `handlers.rs` | Delete all resources |
| GET | `/server/status` | `handlers.rs` | Server container statuses |
| GET | `/servers/{server_name}/logs` | `handlers.rs` | SSE stream of server logs |
| POST | `/search-files` | `handlers.rs` | File search via provider |
| GET | `/files/tree` | `file_handlers.rs` | File tree with git overlay (via `SandboxClient`) |
| GET | `/files/read` | `file_handlers.rs` | Read file content (via `SandboxClient`) |
| POST | `/files/write` | `file_handlers.rs` | Write file (via `SandboxClient`) |
| GET | `/files/list` | `file_handlers.rs` | List files (via `SandboxClient`) |
| POST | `/files/search-by-name` | `file_handlers.rs` | Search files by name (via `SandboxClient`) |

Infrastructure health: `GET /health`, `GET /healthz` (not in OpenAPI).

## K8s Label Convention

All managed resources use:
- `moatless.io/task-id` — task identifier (used for selector queries)
- `moatless.io/user-id` — user identifier (on secrets)
- `app.kubernetes.io/managed-by=moatless` — identifies moatless-managed resources
- `app.kubernetes.io/name=moatless-workspace`
- `app.kubernetes.io/component` — `sandbox`, `storage`, `sandbox-service`

## Leader Election

Two independent leader-elected subsystems use K8s Leases:
- `moatless-pod-watcher` — pod lifecycle watcher (15s default lease)
- `moatless-task-reaper` — unified task reaper (idle-sandbox stop + inactive-task close, 2x check interval lease). Implementation lives in `task/reaper.rs`.

Both use `HOSTNAME` env var as identity.

## Module Map

```
sandbox/
├── mod.rs              # Module declarations
├── models.rs           # Domain types: config, provisioning, K8s status, containers
├── schemas.rs          # API schemas: request/response DTOs (ErrorBody, responses, From impls)
├── error.rs            # SandboxError enum → HTTP responses
├── provider.rs         # SandboxProvider trait (lifecycle + sandbox_client())
├── routes.rs           # Axum router + OpenAPI doc
├── handlers.rs         # Lifecycle handler functions (provision, status, stop, start, etc.)
├── file_handlers.rs    # File operation handlers (tree, read, write, list, search-by-name)
├── client.rs           # SandboxClient gRPC wrapper + classify_grpc_error()
├── notify.rs           # Fire-and-forget gRPC notification
├── jwt.rs              # JWT generation/verification
├── docker/
│   ├── mod.rs           # Docker submodule declarations
│   ├── client.rs        # DockerClient wrapper (bollard)
│   ├── provider.rs      # DockerSandboxProvider (SandboxProvider impl)
│   ├── naming.rs        # Container name generation
│   ├── container_builder.rs  # Docker container config builder
│   ├── status_resolver.rs    # Docker status resolution
│   └── health.rs        # File search via gRPC (Docker)
└── k8s/
    ├── mod.rs           # K8s submodule declarations
    ├── client.rs        # K8sClient wrapper
    ├── provider.rs      # K8sSandboxProvider (SandboxProvider impl)
    ├── naming.rs        # K8s resource name generation (63-char limit)
    ├── pod_builder.rs   # Code-generated Pod specs (sandbox + server containers)
    ├── deployment.rs    # Deployment CRUD + scaling
    ├── service.rs       # ClusterIP Service CRUD
    ├── pvc.rs           # PersistentVolumeClaim CRUD
    ├── ingress.rs       # Traefik IngressRoute CRD CRUD
    ├── pod_status.rs    # Pure pod inspection functions + test helpers
    ├── pod_watcher.rs   # Leader-elected pod lifecycle watcher + event publishing
    ├── status_resolver.rs  # Read-only status resolution (direct K8s API)
    ├── pod_reader.rs    # Container status fetching + log streaming
    ├── leader_election.rs  # K8s Lease-based leader election
    └── auto_provision.rs   # Auto-provision from DB state
```

## Design Patterns

- **Data-driven server configuration**: No hardcoded server names. `ServerConfig` carries all per-server fields (image, commands, env vars, capabilities). `auto_provision.rs` only injects defaults when the DB `servers` column is empty (backwards compat for old repos). Image resolution: `server.docker_image` → `sandbox_image` fallback.
- **K8s API as source of truth**: `StatusResolver` reads status directly from the K8s API (Deployments + Pods). No intermediate caching layer — simplifies the architecture and eliminates stale-read issues.
- **Config-first server status**: Server entries always start from repository config (DB), then overlay K8s container statuses. This ensures servers appear in the UI even before pods exist. The `default` flag and `enable_inspector` field flow from config through snapshots to frontend `ServerEntry`.
- **Event publishing via Redis Streams**: The pod watcher publishes lifecycle events (sandbox, server, agent) to Redis Streams. SSE consumers and event handlers read from these streams — this is separate from status resolution.
- **Pod-keyed tracking**: `pod_watcher` tracks state by pod_name (not task_id) so rolling updates don't confuse old/new pods
- **Idempotent K8s operations**: All create operations handle 409 Conflict gracefully; all delete operations handle 404 NotFound
- **Deterministic naming**: All K8s resource names derived from `task_id` via `naming.rs`, capped at 63 chars

## File Organization Strategy (Backend-Wide)

Each domain module owns its types and persistence layer. Auth persistence lives in `auth/`. The `db.rs` file is just the connection pool.

### Standard Module Structure

```
{module}/
├── mod.rs          # Module declarations
├── models.rs       # Domain types: enums + DB row structs (sqlx::FromRow)
├── dao.rs          # Database queries (sqlx): insert/update/find/delete
├── schemas.rs      # API schemas: request/response DTOs (serde + utoipa)
├── handlers.rs     # HTTP handlers (axum extractors → dao → schemas)
├── routes.rs       # Axum router with OpenAPI annotations
├── error.rs        # Module-specific error enum (thiserror → IntoResponse)
└── ...             # Module-specific files (transform.rs, send.rs, etc.)
```

### Key Distinctions

- **`models.rs`** = owned by the module, used by anyone. Contains domain enums (with `sqlx::Type`/`Encode`/`Decode`), DB row structs (`sqlx::FromRow`), and internal domain types. Use domain names (e.g. `Task`, `Repository`) — no `Row` suffix.
- **`schemas.rs`** = API boundary shapes. Request/response structs with `serde` + `utoipa`. Imports from `models.rs` for conversions.
- **`dao.rs`** = persistence layer. Uses `models.rs` row types + `crate::db::DbPool`. One DAO per module.

### Import Conventions

```rust
// Within the same module — use super::
use super::models::Task;
use super::dao as task_dao;

// Cross-module — use crate::
use crate::task::models::Task;
use crate::task::dao as task_dao;
use crate::db::DbPool;

// Auth persistence lives in auth/
use crate::auth::provider_token_dao;
use crate::auth::users;
```

### What Lives in `db.rs`

Just the connection pool (flat file, not a directory):
- `db::DbPool` type alias + `create_pool()` + `run_migrations()`

### What Lives in `auth/`

Auth subsystem persistence and models:
- `auth/models.rs` — `UserDisplay`, `ProviderAccessToken`, `ProviderToken`, `OAuthTokens`, `ProviderUserInfo`
- `auth/provider_token_dao.rs` — cross-cutting credential resolution, used by 4+ modules
- `auth/users.rs`, `auth/sessions.rs`, `auth/api_keys.rs`, `auth/provider_tokens.rs` — auth queries
- `auth/schemas.rs` — API request/response types (SessionResponse, UserInfo, etc.)

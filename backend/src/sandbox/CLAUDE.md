# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

The `sandbox` module is a Rust (Axum) service that manages the full lifecycle of Kubernetes-based sandbox environments. Each sandbox is a multi-container pod identified by a `task_id`, provisioned on demand, and monitored via a K8s pod watcher. The module runs as part of the `moatless-vibe-backend` binary.

## Architecture

### Provider Abstraction

`SandboxProvider` trait (`provider.rs`) defines the provider-agnostic interface for sandbox operations: provision, start, stop, restart, cleanup, status, health, server logs, and file search. The only implementation is `K8sSandboxProvider` (`k8s/provider.rs`).

### Request Flow

```
HTTP request → routes.rs (Axum OpenApiRouter)
             → handlers.rs (extract Path/Json, delegate to provider)
             → K8sSandboxProvider (orchestrates K8s resources)
             → k8s/* modules (Deployment, Service, PVC, Secret, Ingress)
```

### K8s Resources per Sandbox

Provisioning a sandbox creates these resources (all labeled `moatless.io/task-id={task_id}`):

1. **Deployment** — wraps the pod spec, enables stop (scale to 0) / start (scale to 1) without re-provisioning
2. **Pod** (via Deployment) — contains a `sandbox` container + dynamic server containers (`files-server`, `preview-server`, etc.)
3. **Service** (ClusterIP) — exposes sandbox HTTP (8001), gRPC, and server ports
4. **PVC** — optional persistent workspace storage
5. **Secret** — per-user credentials (shared across tasks)
6. **IngressRoute** (Traefik CRD) — optional, one per server port, pattern: `{task_id}--{port}.{proxy_domain}`

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

### Idle Reaper

`idle_reaper.rs` is a separate leader-elected background task that periodically scans running Deployments, queries latest message timestamps from the DB, and stops idle sandboxes (scale to 0).

### gRPC Communication

`SandboxGrpcClient` (`grpc_client.rs`) calls sandbox pods via gRPC (K8s service DNS): execute-agent, stop-agent, notify-messages, search-files. Uses lazy connection with 5s connect timeout, 10s request timeout.

`notify.rs` provides fire-and-forget convenience for notifying a sandbox that new messages are available.

### Auto-Provisioning

`k8s/auto_provision.rs` enables `start()` to transparently provision a workspace that was never provisioned by reading task + repository from the DB and resolving credentials (provider_tokens table → env var fallback).

### JWT Auth

`jwt.rs` generates/verifies HS256 JWTs (90-day expiry, issuer `moatless-vibe-backend`) that sandbox pods use to authenticate back to the backend.

## Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `ServerConfig` | `types.rs` | Data-driven server definition: name, port, docker_image, start_commands, env_vars, default, enable_inspector |
| `ServerSnapshot` | `types.rs` | Cached server state: config fields + runtime status/error. Built via `from_config(&ServerConfig, status)` |
| `ServerCapabilities` | `types.rs` | Derived from `enable_inspector` via `server_capabilities_from_config()` / `server_capabilities_from_snapshot()` |
| `EnvironmentStatus` | `types.rs` | `not_created`, `initializing`, `ready`, `stopped`, `removing`, `error` |
| `PodPhase` | `types.rs` | K8s pod phase + synthetic `Stopped` (replicas=0), `NotFound` |
| `WorkspaceStatusFullResponse` | `types.rs` | Frontend response with servers, proxy URLs, capabilities |
| `ProvisionRequest` / `ProvisionPlan` | `types.rs` | Provisioning input (repo config, git URL, credentials, env vars) |
| `SandboxError` | `error.rs` | Unified error type, maps to HTTP status codes |
| `ContainerObservedStatus` | `types.rs` | K8s container state mapped to server status strings |

## API Endpoints

All endpoints are under `/api/sandbox/v1/tasks/{task_id}/`:

| Method | Path | Handler |
|--------|------|---------|
| POST | `/provision` | Create sandbox resources |
| GET | `/env-status` | Lightweight environment status |
| GET | `/status` | Full workspace status (env + servers + agent) |
| GET | `/health` | Health check |
| POST | `/stop` | Scale Deployment to 0 |
| POST | `/start` | Scale to 1 (or auto-provision) |
| POST | `/restart` | Delete pods (Deployment recreates) |
| POST | `/cleanup` | Delete all K8s resources |
| GET | `/server/status` | Server container statuses |
| GET | `/servers/{server_name}/logs` | SSE stream of server logs |
| POST | `/search-files` | File search via gRPC |

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
- `moatless-idle-reaper` — idle sandbox reaper (2x check interval lease)

Both use `HOSTNAME` env var as identity.

## Module Map

```
sandbox/
├── mod.rs              # Module declarations
├── types.rs            # All domain types + serde + OpenAPI schemas
├── error.rs            # SandboxError enum → HTTP responses
├── provider.rs         # SandboxProvider trait
├── routes.rs           # Axum router + OpenAPI doc
├── handlers.rs         # HTTP handler functions
├── grpc_client.rs      # gRPC client for sandbox pod calls
├── notify.rs           # Fire-and-forget gRPC notification
├── idle_reaper.rs      # Background idle sandbox detector
├── jwt.rs              # JWT generation/verification
└── k8s/
    ├── mod.rs           # K8s submodule declarations
    ├── client.rs        # K8sClient wrapper
    ├── provider.rs      # K8sSandboxProvider (SandboxProvider impl)
    ├── naming.rs        # K8s resource name generation (63-char limit)
    ├── pod_builder.rs   # Code-generated Pod specs (sandbox + server containers)
    ├── deployment.rs    # Deployment CRUD + scaling
    ├── service.rs       # ClusterIP Service CRUD
    ├── pvc.rs           # PersistentVolumeClaim CRUD
    ├── secret.rs        # Per-user Secret CRUD
    ├── ingress.rs       # Traefik IngressRoute CRD CRUD
    ├── pod_status.rs    # Pure pod inspection functions + test helpers
    ├── pod_watcher.rs   # Leader-elected pod lifecycle watcher + event publishing
    ├── status_resolver.rs  # Read-only status resolution (direct K8s API)
    ├── health.rs        # Container status fetching + log streaming + file search
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

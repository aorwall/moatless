# Preview Environment Architecture

This document explains how preview servers are provisioned, tracked, and exposed across the backend. It is intended for contributors wiring new frameworks or adjusting deployment flows.

## Component Map

| Layer | Responsibility | Key Files |
| --- | --- | --- |
| Workspace orchestration | Creates environments, delegates server lifecycle, chooses clone/worktree storage | `apps/backend/services/workspace.service.ts` |
| Local environment | Spawns preview process, proxies traffic, reports lifecycle updates | `apps/backend/environments/local-environment.ts` |
| Docker/worker environment | Requests a worker container to start/stop the preview server and proxy HTTP | `apps/backend/environments/docker-environment.ts`, `apps/backend/worker-app.ts` |
| Preview core | Starts shell command, streams logs, emits lifecycle sink callbacks | `packages/preview-server/src/services/preview-server.ts` |
| Registry (optional) | Persists preview metadata; backs hostname routing resolver | `packages/preview-server/src/services/preview-server-registry.ts` |
| Hostname proxy | Terminates public hostname and forwards to preview host/port | `packages/preview-server/src/services/hostname-proxy.ts` |

## PreviewServer and Lifecycle Sink

`PreviewServer` encapsulates process management (port assignment, readiness checks, log streaming). Lifecycle events (`register`, `update`, `remove`) are emitted through an optional `PreviewServerLifecycleSink`:

```ts
new PreviewServer(config, logger, eventService?, lifecycleSink?)
```

When no sink is provided (e.g. worker mode) the server functions purely locally. Backend code that needs registry information (hostname routing, multi-tenant dashboards) passes a sink implementation. The in-memory registry (`PreviewServerRegistry`) implements the sink and can be swapped with another backend (e.g. database) by conforming to the same interface.

### Snapshot Fields

Lifecycle snapshots capture:

- `status`, `pid`, `port`, `url`
- `proxyUrl` (port-based or hostname-based proxy)
- `hostname` (when hostname routing is used)
- `inspectorEnabled`, `error`, `metadata`

The registry stores this data via `PreviewServerSnapshot` and `PreviewServerUpdate` types (`packages/preview-server/src/types/registry.ts`).

## WorkspaceService Flow

`WorkspaceService` drives environment selection and workspace storage.

1. **Repository loading** – fetch config, determine clone vs worktree strategy.
2. **Storage decision**
   - `cloneOnNewTask: true` → clone repository into `REPO_DIR` (see `createClonedRepository`).
   - `useGitWorktrees: true` → create a worktree under `WORKTREE_DIR` (default to repo parent).
3. **Environment creation** (`createEnvironment`)
   - Local: `LocalEnvironment` receives the preview lifecycle sink.
   - Docker: `DockerEnvironment` does not implement preview sink; remote workers handle process start.
4. **Server delegation** – workspace-level methods (`startServer`, `stopServer`, `getServerStatus`, `streamServerLogs`, etc.) are pass-throughs to the environment.

## LocalEnvironment Flow

`LocalEnvironment.startServer()` constructs a `PreviewServer` with the sink. Responsibilities:

1. Run setup commands (if any) and emit installing events.
2. Prepare log file in `/tmp` per run.
3. Start shell command with allocated port (`getPort()` from `@moatless/shell-utils`).
4. Wait for HTTP readiness, start optional port-based proxy (inspector injection), and update the sink with proxy URL.
5. Expose helper getters (`getPort`, `proxyRequest`, `getLogs`, etc.).

`stopServer()` kills the shell command, stops the proxy, and updates the sink with `status: stopped` while keeping the allocated port for restarts. `dispose()` performs full cleanup and notifies the sink via `remove`.

## Docker Environment & Worker Flow

When `codeExecution.type === 'docker'` the environment sends HTTP requests to a worker container:

- `DockerEnvironment.startServer()` posts to `/server/start` on the worker.
- `Worker-App` (`apps/backend/src/worker-app.ts`) constructs `PreviewServer` **without** a lifecycle sink, because workers manage a single preview instance and expose it via a fixed proxy port.
- `proxyPreviewRequest` proxies via the worker’s `/server/proxy/*` endpoint.
- Stop/cleanup are delegated to `/server/stop` on the worker.

This separation keeps registry coupling out of worker mode and supports distributed deployment later (replace the in-memory store with a shared backend).

## Registry & Hostname Routing

The registry keeps lightweight metadata in a pluggable store (default in-memory map). `RegistryTargetResolver` adapts registry records into `TargetInfo` for `HostnameProxyService`.

Hostname routing is enabled via environment variables:

- `USE_HOSTNAME_ROUTING=true`
- `PROXY_DOMAIN`, `PROXY_PROTOCOL`, `PROXY_PORT`

In hostname mode, `PreviewServer` skips per-task port proxies and simply reports the canonical hostname (`{serverId}.{PROXY_DOMAIN}`) to the sink. Otherwise, the server spins up a per-preview HTTP proxy with inspector script injection.

## Proxy Options

| Mode | Trigger | Behaviour |
| --- | --- | --- |
| Port-based | Default when `USE_HOSTNAME_ROUTING` is false | Local `PreviewServer` launches a lightweight proxy (`createPreviewProxy`) to inject inspector script and expose `proxyUrl` like `http://127.0.0.1:{proxyPort}` |
| Hostname-based | `USE_HOSTNAME_ROUTING=true` | Registry + hostname proxy service routes `https://{taskId}.{PROXY_DOMAIN}` to the preview process. Inspector injection handled by hostname proxy, so local proxy is skipped. |

`apps/backend/routes/preview-environment.ts` and related handlers detect whether a proxy URL is available from status responses and forward requests accordingly.

## Configuration Summary

### Repository (`@moatless/types`)

```ts
previewConfig: {
  serverStartCommands: string[]
  enableInspector?: boolean
  startupTimeoutSeconds?: number
}
codeExecution: {
  type: 'local' | 'docker'
  envVars?: Record<string, string>
  setupCommands?: string[]
}
cloneOnNewTask?: boolean
useGitWorktrees?: boolean
remoteUrl?: string // required when cloneOnNewTask is true
```

### Environment Variables

| Variable | Purpose |
| --- | --- |
| `REPO_DIR` | Root directory for cloned repositories |
| `WORKTREE_DIR` | Root directory for git worktrees |
| `USE_HOSTNAME_ROUTING` | Enable hostname proxy mode |
| `PROXY_DOMAIN`, `PROXY_PROTOCOL`, `PROXY_PORT` | Hostname proxy configuration |
| `DATA_DIR` | Base storage path for logs/repos/snapshots |

## Testing

Relevant coverage:

- `apps/backend/src/services/__tests__/workspace.unit.test.ts` – unit coverage of workspace interactions and sink wiring.
- `apps/backend/src/services/__tests__/workspace.service.integration.test.ts` – clone-based integration flow.
- `apps/backend/src/services/__tests__/workspace-newbranch.unit.test.ts` – worktree/new-branch scenarios.
- `apps/backend/src/__tests__/preview-server-full.integration.test.ts` – full pipeline running a real dev server with preview registry.
- `apps/backend/src/__tests__/port-allocation.integration.test.ts` – ensures unique port allocation across clones.
- `packages/preview-server` builds via `bunx turbo run build --filter=@moatless/preview-server`.

## Extending the System

1. **Integrate a new environment type** – implement `BaseEnvironment` methods and wire it in `WorkspaceService.createEnvironment`, passing the lifecycle sink if it can report preview metadata.
2. **Swap registry storage** – implement `PreviewServerLifecycleSink` (e.g. write to Redis/Postgres) and provide it via `createWorkspaceService`/service container.
3. **Add routing modes** – create a new target resolver that consumes lifecycle data and register it with `HostnameProxyService.getInstance` or a custom proxy.
4. **Instrument analytics** – listen to lifecycle sink events to broadcast preview availability, failure reasons, or to schedule automatic teardown.

## Troubleshooting

- **Preview never reports ready** – confirm the command listens on the injected `PORT`; check logs via `workspaceService.getServerLogs`.
- **Hostname routing fails** – ensure the registry stores `port` and `inspectorEnabled` fields, environment variables are set, and `HostnameProxyService` is started.
- **Worker mode issues** – worker runs without a lifecycle sink; metadata will not appear in the registry. For debugging, attach a sink if you need remote state.
- **Cloned repositories piling up** – `WorkspaceService.cleanupWorkspace` removes clones when `cloneOnNewTask` is enabled. Integration tests mimic this by deleting clone folders between runs.

Use this architecture overview when adjusting preview flows, swapping storage, or integrating additional runtime environments.

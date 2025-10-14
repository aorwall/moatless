# Workspace & Preview System Redesign Proposal

## 1. Context Snapshot
- Workspace orchestration lives in `apps/backend/src/services/workspace.service.ts`, which currently reuses cached `Workspace` instances without validating repository drift.
- Local runtime coordination happens in `apps/backend/src/environments/local-environment.ts` for setup commands and preview lifecycle, while Docker-specific logic sits in `apps/backend/src/environments/docker-environment.ts`.
- Preview execution and proxying are driven by `packages/preview-server/src/services/preview-server.ts`, which replays setup commands on each start.
- Task tools (`apps/backend/src/services/tool.service.ts`) manually chain workspace and environment APIs to run installs or restart the preview server.

## 2. Pain Points
1. **Stale configuration reuse**: cached workspaces are returned without comparing the stored repository configuration with the latest definition, so updated env vars or commands never propagate.
2. **Partial refresh logic**: `LocalEnvironment.refreshConfiguration` only stops the preview server and swaps the config reference; install state, timestamps, and setup flags remain stale, undermining `configOutdated` checks.
3. **Duplicated setup execution**: environments mark setup commands as completed after the first run, while the preview server repeats those commands during every start, leading to redundant installs and conflicting lifecycle events.

## 3. Target Architecture

### 3.1 Workspace Runtime Record
- **Purpose**: Provide a lightweight cache of environment state so callers can reuse workspaces without expensive rehydration.
- **Location**: `apps/backend/src/workspace/runtime.ts`
- **Stored fields** (per task):
  - `taskId`, `repositoryId`, `workspacePath`
  - `workspaceState` (`created | ready | error`, lastValidatedAt, lastError)
  - `previewState` (status, last known URL, PID, proxy/port info, lastStartedAt)
- **Key operations**:
  1. `load(taskId)` → returns runtime snapshot or `null`.
  2. `save(snapshot)` → atomic upsert after mutations.
  3. `markWorkspaceReady(taskId, workspacePath)` → called after `workspace.setup()` verifies path/container readiness.
  4. `updatePreviewState(taskId, state)` → invoked by the preview manager on lifecycle changes.
- **Flow integration**:
  - Types and helpers defined in `apps/backend/src/workspace/runtime.ts`.
  - Persist runtime snapshots through the existing storage layer (`apps/backend/src/storage/storage-backend.ts`), keeping implementation aligned with current task persistence patterns.
  - `WorkspaceService.getOrCreateWorkspace()` loads the runtime to confirm a workspace path exists; if missing, it rebuilds the workspace and updates the record.
  - Preview operations update `previewState`, enabling the UI and tools to query status without touching the underlying environment objects.

### 3.2 Environment Readiness Checks
- **Local environment**: `LocalEnvironment.setup()` remains a quick validation that the workspace path exists and is accessible. No repository setup commands run at this stage.
- **Docker environment**: `DockerEnvironment.setup()` focuses on ensuring the worker container is running, ports are mapped, and filesystem sync is complete. It does not execute project-specific scripts.
- **Rationale**: By keeping environment setup lightweight, workspace creation stays fast and preview launches can decide when (or if) to run heavier setup routines.

### 3.3 Preview Server Manager
- **Purpose**: Centralize preview lifecycle and avoid duplicating process/proxy logic across environments and workspace service.
- **Location**: `apps/backend/src/workspace/preview-server-manager.ts`
- **Responsibilities**:
  - Manage a single preview lifecycle per task, regardless of environment type (local/docker).
  - Resolve the workspace via `WorkspaceService.getOrCreateWorkspace(taskId)` and use the returned `Workspace` to access environment + metadata before any preview action.
  - Fetch fresh repository configuration from `RepositoryService` on every start, apply env placeholders, and optionally run repository `setupCommands` immediately before booting the preview process. These commands execute only when launching (or relaunching) the preview server, ensuring the workspace is prepared on-demand.
  - Delegate runtime-specific operations to the environment (`startPreview`, `stopPreview`, `getPreviewStatus`, `streamPreviewLogs`).
  - Persist `previewState` (status, PID, URL, proxy metadata) back into the runtime after each transition.
- **API sketch**:
  ```ts
  interface PreviewServerManager {
    start(taskId: string, options?: { reason?: string }): Promise<ServerStatus>
    stop(taskId: string, options?: { reason?: string }): Promise<void>
    getStatus(taskId: string): Promise<ServerStatus>
    streamLogs(taskId: string): AsyncGenerator<LogEvent>
  }
  ```
- **Interaction notes**:
  - Preview HTTP routes (`apps/backend/src/routes/preview-environment.ts`) and TaskService utilities (e.g., screenshot capture) call the manager directly with `taskId`; no intermediate workspace preview helper remains.
  - Manager reuses `WorkspaceService` internally to hydrate the workspace/environment and update runtime snapshots.
  - Retrieves the latest repository config from `RepositoryService` during every `start` call, then executes any configured `setupCommands` before invoking `environment.startPreview()`.
  - Publishes lifecycle events (`starting`, `started`, `failed`, `stopped`) via EventService and streams process output/logs to clients via `LogHub`.

### 3.4 End-to-End Flow Overview

#### Task creation & workspace provisioning
```
Client/TaskService → WorkspaceService                Workspace Module
        │                     │ load runtime snapshot
        │                     ├─ missing? clone/worktree + env.setup()
        │                     │   ↳ mark runtime ready (workspaceState=ready)
        │                     └─ existing? reuse cached workspace
        ▼                                         ▼
   task created                          workspace ready (path, env)
```

#### Preview start (routes or TaskService)
```
Preview caller (route/tool) → PreviewServerManager.start(taskId)
        │                                │
        │                                ├─ WorkspaceService.getOrCreateWorkspace(taskId)
        │                                │     ↳ returns Workspace (path + environment)
        │                                │
        │                                ├─ RepositoryService.getRepositoryById()
        │                                ├─ run repository.setupCommands via environment.runCommand()
        │                                ├─ environment.startPreview(launchConfig)
        │                                ├─ runtime.updatePreviewState(taskId, ...)
        │                                └─ LogHub/EventService stream logs + lifecycle events
        ▼
    ServerStatus/Log stream → caller
```

#### Preview stop & restart
- `PreviewServerManager.stop(taskId)` follows the same pattern: load workspace, invoke `environment.stopPreview()`, update runtime snapshot, stream shutdown logs.
- Restart is simply another `start` call; the manager fetches fresh config, re-runs setup commands if defined, and calls `environment.startPreview()` again.
- Because both HTTP routes and TaskService utilities call the manager directly, every consumer observes identical lifecycle events and log streams.

### 3.4 Component Interaction Diagram
```
┌─────────────┐      config query         ┌──────────────┐
│ Repository  │ ◄──────────────────────── │ Preview      │
│  Service    │                           │ Server Mgr   │
└─────▲───────┘                           └──────┬───────┘
      │                                         │
      │                                         │ start/stop preview
      │                                         │
┌─────┴────────┐  ensure env ready       ┌──────▼────────┐
│ Workspace    │ ───────────────────────►│ Local/Docker  │
│ Service      │                          │ Environment   │
└─────┬────────┘                          └──────┬────────┘
      │ updates runtime                           │ validates path/container
      │                                           │ exposes preview ops
┌─────▼────────┐   control + status       ┌───────▼──────────────┐
│ Preview      │ ───────────────────────► │ Preview Server /     │
│ Server Mgr   │                          │ Container Worker     │
└─────┬────────┘                          └───────┬──────────────┘
      │ stream logs directly                       │ stdout/stderr or worker logs
      │ status queries                             │
      └────────────────────────────────────────────┘
```
- Preview API routes call the manager with `taskId`, keeping HTTP orchestration thin.
- `PreviewServerManager` uses `WorkspaceService` to ensure the environment is ready, fetches repository config, and delegates runtime-specific work to the environment.
- Runtime snapshots and log streaming stay centralized in the manager so all callers (routes, TaskService, automation) see consistent behavior.

#### Preview Request Flow Summary
1. Client sends request to `/preview` routes.
2. Route invokes `PreviewServerManager` (`start`/`stop`/`getStatus`/`streamLogs`).
3. Manager calls `WorkspaceService.getOrCreateWorkspace(taskId)` to hydrate the workspace and environment.
4. Manager fetches repository config, runs optional setup commands, and delegates to the environment preview API.
5. Manager updates runtime snapshot, pushes log events to `LogHub`, and emits lifecycle events.
6. Route streams status/log responses back to the caller.

## 4. Component Responsibilities

| Component | Core APIs | Data Managed | Location | Notes |
| --- | --- | --- | --- | --- |
| `WorkspaceRuntime` | `load`, `save`, `markWorkspaceReady`, `updatePreviewState`, `delete` | Serialized runtime snapshot (workspace path, workspace state, preview state) | `apps/backend/src/workspace/runtime.ts` | Types and helpers reused by services to reason about task environments without rehydrating full workspaces. |
| `Workspace` | `setup`, `getEnvironment`, `getWorkspacePath`, file/git helpers | Workspace path, file operations service, environment reference | `apps/backend/src/workspace/workspace.ts` | Provides access to filesystem/command operations; no longer owns preview methods (manager calls environment directly). |
| `PreviewServerManager` | `start`, `stop`, `getStatus`, `streamLogs` | Preview process status, proxy metadata, restart counters, last setup-command result | `apps/backend/src/workspace/preview-server-manager.ts` | Exposed directly to preview routes/TaskService; ensures workspace readiness via `WorkspaceService`, fetches fresh repo config, runs optional `setupCommands`, delegates to environment preview ops, and streams logs via `LogHub`. |
| `LogHub` | `append`, `stream`, `tail`, `rotate` | Structured log entries tagged by source/channel | `apps/backend/src/utils/task-log.ts` (extended) | Centralizes task/preview output so API routes and tooling receive consistent log streams. |
| Environment classes (`LocalEnvironment`, `DockerEnvironment`) | `setup`, `runCommand`, `createAgent`, `teardown`, `startPreview`, `stopPreview`, `getPreviewStatus`, `streamPreviewLogs` | Environment-specific handles (workspace path, container info) + preview adapters | `apps/backend/src/environments/` | Perform lightweight readiness checks (path exists, container up) and expose preview operations that the manager calls. |

## 5. Migration Plan
1. Introduce the `WorkspaceRuntime` persistence layer and update `WorkspaceService` to consult it before returning cached workspaces.
2. Implement `PreviewServerManager`, consolidate preview start/stop logic there, and execute any repository `setupCommands` directly within the manager's launch flow.
3. Update `LocalEnvironment` and `DockerEnvironment` to perform lightweight readiness checks only, delegating preview control to the manager.
4. Add regression tests to cover preview launches with fresh repository configuration, runs with/without setup commands, concurrent restarts, and log streaming.

## 6. Implementation Checklist

The full workspace/preview stack is reimplemented under `apps/backend/src/workspace` and then adopted by existing routes/services.

### 6.1 Build new workspace module
- [ ] Create `apps/backend/src/workspace/runtime.ts` with `WorkspaceRuntimeSnapshot` types, validation, and persistence helpers.
- [ ] Create `apps/backend/src/workspace/preview-server-manager.ts` implementing `start`, `stop`, `getStatus`, `streamLogs`, setup command execution, and runtime updates.
- [ ] Refactor `apps/backend/src/workspace/workspace.ts` to depend on runtime helpers and preview manager (remove legacy preview code).
- [ ] Add `apps/backend/src/workspace/index.ts` exporting the new module.
- [ ] Add tests under `apps/backend/src/workspace/__tests__` for runtime helper, preview manager, and workspace orchestration.

**Snapshot schema**
```ts
interface WorkspaceRuntimeSnapshot {
  taskId: string
  repositoryId: string
  workspacePath: string | null
  workspaceState: {
    status: 'created' | 'ready' | 'error'
    lastValidatedAt?: string
    lastError?: string
  }
  previewState: {
    status: 'stopped' | 'starting' | 'started' | 'failed'
    url?: string
    proxyUrl?: string
    pid?: number
    lastStartedAt?: string
    lastStoppedAt?: string
    lastError?: string
  }
}
```

### 6.2 Reinforce storage & environment support
- [ ] Extend `apps/backend/src/storage/storage-backend.ts` (and implementations) with runtime snapshot persistence.
- [ ] Update `apps/backend/src/environments/base-environment.ts` to include preview interface; implement preview methods in `local-environment.ts` and `docker-environment.ts`.
- [ ] Ensure environments emit consistent status/log events consumed by the preview manager.

### 6.3 Integrate with WorkspaceService & TaskService
- [ ] Update `apps/backend/src/services/workspace.service.ts` to construct the new `Workspace`, rely on runtime helpers, and drop old preview methods.
- [ ] Update `TaskService` and background workflows (e.g., screenshot capture) to call `PreviewServerManager` instead of legacy workspace APIs.
- [ ] Replace any in-memory preview state tracking with runtime snapshot usage.

### 6.4 Switch API routes and tooling
- [ ] Update `apps/backend/src/routes/preview-environment.ts` to call `PreviewServerManager` directly (start/stop/status/logs).
- [ ] Update `apps/backend/src/services/tool.service.ts` and other consumers to use preview manager APIs.
- [ ] Enhance `apps/backend/src/utils/task-log.ts` to support channel-aware log streaming/retention for preview output.

### 6.5 Testing & migration tasks
- [ ] Add regression tests covering preview launches with fresh repository config, runs with/without setup commands, concurrent restarts, and log streaming.
- [ ] Add smoke tests validating local vs docker readiness checks.
- [ ] Plan data migration (if needed) to backfill runtime snapshots for existing tasks.
- [ ] Document new preview lifecycle APIs for internal tooling/agents.

## 7. Workspace Module Consolidation (`apps/backend/src/workspace`)

All workspace-related components are consolidated into a single cohesive module at `apps/backend/src/workspace`:

```
apps/backend/src/workspace/
├── runtime.ts                    # Runtime snapshot types and helpers
├── preview-server-manager.ts     # Preview lifecycle management
├── workspace.ts                  # Core workspace class (existing, updated)
├── index.ts                      # Module exports
└── __tests__/
    ├── runtime.test.ts
    ├── preview-server-manager.test.ts
    └── workspace-integration.test.ts
```

### 7.1 Target Workspace Object Model
- Refactor `Workspace` (`workspace.ts`) to wrap:
  - `FileOperations` (existing functionality).
  - `environment` implementing the enhanced preview interface.
  - `runtimeStorage` helper for reading/writing `WorkspaceRuntimeSnapshot`.
- Remove embedded preview logic from `Workspace`; it simply exposes environment/file helpers and leaves preview orchestration to `PreviewServerManager`.
- Keep `Workspace.setup()` focused on environment readiness checks (path/container validation) and update runtime snapshot (`workspaceState.status = 'ready'`).

### 7.2 Environment Interface Upgrade
- Introduce `PreviewCapableEnvironment` in `apps/backend/src/environments/base-environment.ts` and have `BaseEnvironment` implement it so every concrete environment inherits the preview contract:
  ```ts
  interface PreviewCapableEnvironment {
    startPreview(options: PreviewStartOptions): Promise<PreviewLaunchResult>
    stopPreview(): Promise<boolean>
    getPreviewStatus(): Promise<ServerStatus>
    streamPreviewLogs(): AsyncGenerator<LogStreamChunk>
  }
  ```
- Ensure `BaseEnvironment` implements default stubs (throwing `NotImplementedError`) so subclasses must override with concrete behavior.
- Update `LocalEnvironment` to provide the preview operations by delegating to its `PreviewServer` instance.
- Update `DockerEnvironment` to provide the preview operations by calling the worker HTTP API and adapting responses/log streams.
- Ensure both environments surface telemetry (status/log chunks) in a consistent shape for the manager.

### 7.3 Workspace Service & Route Wiring
- `WorkspaceService` continues to own workspace construction/setup and persists runtime snapshots; it no longer exposes preview helpers.
- `PreviewServerManager` depends on `WorkspaceService` to obtain a workspace/environment whenever a preview action is requested.
- Update preview API routes (`apps/backend/src/routes/preview-environment.ts`) and TaskService utilities to call the manager directly.
- Remove any redundant preview state tracking inside `WorkspaceService`; rely on runtime snapshots + manager feedback.

### 7.4 File-Level Tasks
- `apps/backend/src/workspace/runtime.ts`: define `WorkspaceRuntimeSnapshot` types, create helper functions for snapshot manipulation and validation.
- `apps/backend/src/workspace/preview-server-manager.ts`: implement manager with full lifecycle control, setup command execution, direct log streaming, and runtime updates.
- `apps/backend/src/workspace/workspace.ts`: update to use manager for preview operations and runtime storage for state persistence.
- `apps/backend/src/workspace/index.ts`: export all workspace module components with clean public API.
- `apps/backend/src/workspace/__tests__/*`: add comprehensive test coverage for all module components including integration tests.

### 7.5 Cleanup
- Delete obsolete preview helper functions left over from the old implementation.
- Audit for TODOs referencing deprecated preview flows and update them.
- Run lint/tests to ensure TypeScript declarations consumed by API routes remain accurate.

## 8. Open Questions
- None at this time.

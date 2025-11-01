# Preview Feature Architecture

## Purpose
Manages preview server lifecycle (start/stop/restart), HTTP proxy with inspector injection, and state persistence. Integrates with workspace provisioning while keeping preview logic independent from workspace concerns.

## Layer Responsibilities

| Layer | Components | Purpose |
|-------|-----------|---------|
| **API** | `preview.routes.ts` | REST + SSE endpoints for preview control, status, log streaming |
| **Orchestration** | `PreviewOrchestratorService` | Coordinates workspace, repository config, runtime, and snapshot updates |
| **Runtime** | `PreviewRuntimeService`<br>`PreviewServer` | Process lifecycle (start/stop/restart)<br>Shell command execution, HTTP polling |
| **Proxy** | `PreviewProxyService`<br>`RegistryTargetResolver` | Per-task HTTP proxy<br>Hostname resolution for subdomain routing |
| **State** | `PreviewRegistryService`<br>`PreviewStateBridgeService` | Lifecycle sink + persistence<br>Workspace integration interface |
| **Inspector** | `html-injector`<br>`inspector-script`<br>`assets/` | Script injection utilities<br>Inspector JS generation<br>Client-side inspector code |
| **Storage** | `PreviewRegistryStorage` | File-based persistence via `StorageBackend` |

## Service Architecture

```
┌─────────────────────────────────────────────────────┐
│ Preview Routes (API)                                │
│  - POST /server/start                               │
│  - POST /server/stop                                │
│  - POST /server/restart                             │
│  - GET  /server/status                              │
│  - GET  /server/logs/stream (SSE)                   │
└─────────────────────────┬───────────────────────────┘
                          │
           ┌──────────────▼──────────────┐
           │ PreviewOrchestratorService  │
           │  - Workspace integration    │
           │  - Repository config        │
           │  - Snapshot updates         │
           └──────┬──────────────┬───────┘
                  │              │
        ┌─────────▼──────┐  ┌───▼──────────────────┐
        │ PreviewRuntime │  │ PreviewStateBridge   │
        │ Service        │  │ Service              │
        │  - Process map │  │  - Update snapshots  │
        │  - Start/stop  │  │  - Workspace state   │
        │  - Log stream  │  └──────────────────────┘
        └─────────┬──────┘
                  │
        ┌─────────▼──────────────────────┐
        │ PreviewServer (runtime/)       │
        │  - ShellCommand execution      │
        │  - HTTP readiness polling      │
        │  - Lifecycle events            │
        └────┬──────────────┬────────────┘
             │              │
   ┌─────────▼──────┐  ┌───▼──────────────┐
   │ PreviewRegistry│  │ PreviewProxy     │
   │ Service        │  │ Service          │
   │  - Lifecycle   │  │  - HTTP proxy    │
   │  - Persistence │  │  - Inspector     │
   └────────────────┘  │    injection     │
                       └──────────────────┘
```

## Key Interactions

### 1. Start Preview
```
Route → Orchestrator.start(taskId, repositoryId)
  ├─> WorkspaceRuntimeManager.ensureWorkspace()
  ├─> RepositoryService.getRepository()
  ├─> LocalEnvironmentDriver.ensureTaskLog(taskId)
  ├─> RuntimeService.start(config)
  │    ├─> PreviewServer.start()
  │    │    ├─> ShellCommand (bun run preview)
  │    │    └─> HTTP polling (readiness)
  │    └─> RegistryService.register(serverId, snapshot)
  │         └─> RegistryStorage.save(record)
  └─> StateBridge.updatePreviewState(status, endpoint)
       └─> WorkspaceRuntimeManager.updateSnapshot()
```

### 2. Stop Preview
```
Route → Orchestrator.stop(taskId)
  └─> RuntimeService.stop(taskId)
       ├─> PreviewServer.stop()
       │    └─> Kill shell process
       ├─> RegistryService.update(serverId, {status: 'stopped'})
       └─> StateBridge.updatePreviewState({status: 'stopped'})
```

### 3. Log Streaming
```
SSE Route → Orchestrator.getLogs(taskId, limit)
  ├─> RuntimeStorage.load(taskId) → workspace path
  ├─> LocalEnvironmentDriver.ensureTaskLog(taskId) → log path
  └─> RuntimeService.streamLogs(taskId)
       └─> Async generator from unified task log
```

## Unified Logging Strategy

Preview reuses workspace logging infrastructure:
- `LocalEnvironmentDriver.ensureTaskLog(taskId)` provides single log path
- Setup commands (`bun install`) write to this log
- Preview server output appends to same log
- SSE endpoints stream from unified log file
- No separate `PreviewLogService` needed

## Inspector Integration

Preview injects real-time inspection tools into HTML responses:

```
HTTP Request → PreviewProxyService
  ├─> Detect HTML response
  ├─> Inject inspector script via html-injector
  │    ├─> <script> tag with inspector-script.js
  │    └─> Component selection, console capture
  └─> Return modified HTML
```

Inspector client features:
- Element selection with component ID tracking
- Console log streaming to backend
- Network request monitoring
- Navigation tracking
- Screenshot coordination

## State Management

### Preview Server Record
Persisted by `PreviewRegistryService`:
```typescript
{
  serverId: string          // Same as taskId
  status: ServerStatusType  // 'stopped' | 'starting' | 'started' | 'failed'
  port?: number
  url?: string
  proxyUrl?: string
  hostname?: string
  pid?: number
  error?: string
  metadata?: Record<string, unknown>
  inspectorEnabled: boolean
  createdAt: Date
  updatedAt: Date
}
```

### Workspace Runtime Snapshot
Updated via `PreviewStateBridge`:
```typescript
{
  preview: {
    status: ServerStatusType
    endpoint?: string
    port?: number
    lastStartedAt?: Date
    lastError?: string
  }
}
```

## Dependency Injection

All services use InversifyJS with `@injectable()`:
```typescript
@injectable()
export class PreviewOrchestratorService {
  constructor(
    @inject(PreviewRuntimeService) runtime,
    @inject(PreviewRegistryService) registry,
    @inject(RepositoryService) repositoryService,
    @inject(WorkspaceRuntimeManager) runtimeManager,
    @inject(EventService) eventService
  ) {}
}
```

## Integration with Workspace

Preview feature consumes workspace services:
- **WorkspaceRuntimeManager** - Snapshot persistence
- **LocalEnvironmentDriver** - Command execution, unified logging
- **WorkspaceCoordinator** - Workspace provisioning

Workspace feature consumes preview via bridge:
- **PreviewStateBridge** - Update preview state in snapshots
- Avoids circular dependencies

## Event Streaming

Preview emits `ServerEvent` through `EventService`:
```typescript
{
  type: 'server'
  subType: 'starting' | 'started' | 'stopped' | 'failed'
  taskId: string
  payload: {
    status: ServerStatusType
    port?: number
    url?: string
    error?: string
  }
}
```

## File Structure

```
features/preview/
  api/
    preview.routes.ts          # REST + SSE endpoints
  services/
    preview-orchestrator.service.ts   # High-level coordination
    preview-runtime.service.ts        # Process lifecycle
    preview-registry.service.ts       # Lifecycle sink + persistence
    preview-proxy.service.ts          # HTTP proxy + injection
    preview-state-bridge.service.ts   # Workspace integration
    preview-state-bridge.interface.ts # Bridge contract
  runtime/
    preview-server.ts                 # Shell-based server
    registry-target-resolver.ts       # Hostname resolution
  inspector/
    html-injector.ts           # Script injection
    inspector-helpers.ts       # DOM utilities
    inspector-script.ts        # JS generation
    assets/
      preview-inspector.js     # Client code
  storage/
    preview-registry.storage.ts  # Persistence adapter
  types/
    index.ts                   # Shared types
  utils/
    proxy-core.ts              # Proxy helpers
  index.ts                     # Feature exports
  README.md                    # Feature documentation
```

## Dependencies

- `@moatless/shell-utils` - Shell command execution
- `@moatless/types` - Shared type definitions
- `inversify` - Dependency injection framework
- Workspace feature - Execution primitives, unified logging
- Repository feature - Repository configuration

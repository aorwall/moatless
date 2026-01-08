# Preview Feature Architecture

## Purpose
Manages preview server lifecycle (start/stop/restart), HTTP proxy with inspector injection, and state persistence. Integrates with workspace provisioning while keeping preview logic independent from workspace concerns.

## Layer Responsibilities

| Layer | Components | Purpose |
|-------|-----------|---------|
| **API** | `preview.routes.ts` | REST + SSE endpoints for preview control, status, log streaming |
| **Orchestration** | `PreviewOrchestratorService` | Coordinates workspace, repository config, runtime, and snapshot updates |
| **Runtime** | `PreviewRuntimeService`<br>`PreviewServer` | Process lifecycle (start/stop/restart)<br>Shell command execution, HTTP polling |
| **Proxy** | `PreviewProxyService`<br>`HostnameProxyService`<br>`RegistryTargetResolver` | Per-task HTTP proxy (port-based routing)<br>Global subdomain proxy (hostname-based routing)<br>Resolves taskId → preview server port + inspector script |
| **State** | Workspace snapshots | Preview state persisted in `WorkspaceRuntimeManager` snapshots (no separate preview_servers table) |
| **Inspector** | `html-injector`<br>`inspector-script`<br>`assets/` | Script injection utilities<br>Inspector JS generation<br>Client-side inspector code |

## Service Architecture

```
┌─────────────────────────────────────────────────────┐
│ Preview Routes (API)                                │
│  - POST /server/start                               │
│  - POST /server/stop                                │
│  - POST /server/restart                             │
│  - GET  /server/status                              │
│  - GET  /server/logs (NDJSON streaming)            │
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

## Proxy Modes

Preview supports two proxy modes for routing traffic to preview servers:

### Port-Based Routing (Default)
Each task gets its own proxy server on a unique port.

**Configuration:**
- No environment variables required (default behavior)
- Automatic port allocation for each preview server

**URL Format:**
```
http://localhost:3001  (task 1)
http://localhost:3002  (task 2)
```

**Advantages:**
- Simple setup, works out of the box
- No DNS/domain configuration needed
- Good for local development

### Hostname-Based Routing (Optional)
All tasks share one global proxy server with subdomain routing.

**Configuration:**
Environment variables:
```bash
USE_HOSTNAME_ROUTING=true
PROXY_PORT=80              # Port for global proxy (default: 80)
PROXY_DOMAIN=proxy.moatless.xyz  # Base domain (default: proxy.moatless.xyz)
PROXY_PROTOCOL=http        # http or https (default: http)
```

**URL Format:**
```
http://task-123.proxy.moatless.xyz
http://task-456.proxy.moatless.xyz
```

**How It Works:**
1. `HostnameProxyService` starts on `PROXY_PORT` when app initializes
2. Extracts subdomain (taskId) from incoming requests
3. Uses `RegistryTargetResolver` to look up preview server port from workspace snapshots
4. Proxies request to `localhost:<port>` with inspector script injection

**Advantages:**
- Consistent URLs across deployments
- Single port for all preview servers
- Better for production deployments

**Status Response:**
The `/api/v1/tasks/:taskId/server/status` endpoint returns the appropriate URL based on the active mode:
- Port-based: `{ "url": "http://localhost:3001" }`
- Hostname-based: `{ "url": "http://task-123.proxy.moatless.xyz" }`

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
    preview.routes.ts                 # REST + SSE endpoints
  services/
    preview-orchestrator.service.ts   # High-level coordination
    preview-runtime.service.ts        # Process lifecycle
    preview-proxy.service.ts          # Per-task HTTP proxy (port-based)
    hostname-proxy.service.ts         # Global subdomain proxy (hostname-based)
  runtime/
    preview-server.ts                 # Shell-based server management
    registry-target-resolver.ts       # TaskId → port + inspector resolution
  inspector/
    html-injector.ts                  # Script injection utilities
    inspector-helpers.ts              # DOM utilities
    inspector-script.ts               # JS generation
    assets/
      preview-inspector.js            # Client-side inspector code
  types/
    index.ts                          # Shared types
    config.ts                         # Configuration types
  utils/
    proxy-core.ts                     # Proxy helper functions
  index.ts                            # Feature exports
  README.md                           # Feature documentation
```

## Dependencies

- `@moatless/shell-utils` - Shell command execution
- `@moatless/types` - Shared type definitions
- `inversify` - Dependency injection framework
- Workspace feature - Execution primitives, unified logging
- Repository feature - Repository configuration

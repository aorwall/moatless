# Preview Feature

This feature owns preview server lifecycle management, including runtime process control, proxy services, inspector tooling, and state persistence.

## Architecture

### Service Layers

1. **PreviewOrchestratorService** - High-level coordination
   - Integrates with workspace feature via `PreviewStateBridge`
   - Coordinates repository config, workspace provisioning, and runtime
   - Updates workspace snapshots with preview state

2. **PreviewRuntimeService** - Process lifecycle management
   - Manages active `PreviewServer` instances (map keyed by taskId)
   - Start/stop/restart operations
   - Log streaming from unified task logs

3. **PreviewRegistryService** - Lifecycle sink & persistence
   - Implements `PreviewServerLifecycleSink` interface
   - Persists preview server records
   - Provides lookup for hostname resolution

4. **PreviewProxyService** - HTTP proxy with script injection
   - Per-task proxy servers
   - Injects inspector scripts into HTML responses

5. **PreviewStateBridgeService** - Workspace integration
   - Minimal interface for workspace to update preview state
   - Avoids circular dependencies

### Runtime Layer

- **PreviewServer** - Core server process management
  - Shell command execution
  - HTTP readiness polling
  - Lifecycle event emission
  - Log file management

- **RegistryTargetResolver** - Hostname resolution
  - Resolves subdomain → port + script for hostname proxy

### Inspector Module

- **html-injector** - Script injection utilities
- **inspector-helpers** - Component selection helpers
- **inspector-script** - Inspector JavaScript generation
- **assets/** - Static inspector client code

### Storage

- **PreviewRegistryStorage** - Persistence adapter
  - Uses `StorageBackend` for file-based storage
  - Schema: `PreviewServerRecord` with timestamps

## Unified Logging Strategy

Preview leverages the workspace feature's unified logging:

1. `LocalEnvironmentDriver.ensureTaskLog(taskId)` provides log path
2. Setup commands write to this log
3. Preview runtime writes to same log
4. SSE endpoints stream from unified log
5. No separate preview log service needed

## Integration Points

### Workspace Feature

- `LocalEnvironmentDriver` provides `ensureTaskLog()` and executes setup commands
- `WorkspaceRuntimeManager` updates snapshots via `PreviewStateBridge`
- Workspace no longer owns preview-specific code

### DI Container

All services registered with `@injectable()` decorator:
- `PreviewOrchestratorService`
- `PreviewRuntimeService`
- `PreviewRegistryService`
- `PreviewProxyService`
- `PreviewStateBridgeService`

## API Routes

Located in `api/preview.routes.ts`:
- `POST /api/v1/tasks/:taskId/server/start`
- `POST /api/v1/tasks/:taskId/server/stop`
- `POST /api/v1/tasks/:taskId/server/restart`
- `GET /api/v1/tasks/:taskId/server/status`
- `GET /api/v1/tasks/:taskId/server/logs` (NDJSON streaming)

## Migration Status

See `MIGRATION_STATUS.md` for detailed progress tracking.

## Testing

Tests organized by layer:
- `api/__tests__/` - Route tests
- `services/__tests__/` - Service unit tests
- `runtime/__tests__/` - Runtime layer tests
- `storage/__tests__/` - Storage tests
- `inspector/__tests__/` - Inspector utility tests
- `__tests__/` - Integration tests

## Dependencies

- `@moatless/shell-utils` - Shell command execution
- `@moatless/types` - Shared types
- `inversify` - Dependency injection
- Workspace feature - Execution primitives, unified logging

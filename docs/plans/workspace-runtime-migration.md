# Workspace Runtime Migration Plan

## Migration Approach
**Build a new workspace feature from scratch** following `docs/patterns/backend-feature-architecture.md`, then migrate existing services to use it and delete legacy code. This is a clean-break migration with no backward compatibility layer.

## Scope
Create a new `features/workspace` module implementing the refactored runtime architecture with `WorkspaceCoordinator`, `WorkspaceRuntimeStore`, and worker-aware environment drivers. The new feature will:

1. **Include all workspace-related code**:
   - Services: `WorkspaceService` → `WorkspaceCoordinator`, `PreviewManager`, `AgentController`, etc.
   - Environments: Move `environments/` directory into `features/workspace/drivers/`
   - Workspaces: Move `workspaces/` directory into `features/workspace/core/`
   - Storage: New runtime snapshot persistence layer

2. **Follow backend feature architecture**:
   - Feature-centric vertical slice under `features/workspace/`
   - InversifyJS dependency injection throughout
   - Explicit ports/adapters separation
   - Colocated routes, services, storage, types

## Current State Analysis
- `services/workspace.service.ts` - 1191 lines, owns everything: repo resolution, provisioning, environment creation, command/preview execution, screenshot capture, background task tracking
- `services/task.service.ts` - Mixes task lifecycle with workspace readiness (setup, preview restarts, screenshot orchestration)
- `environments/` - Environment drivers (local, docker) with setup, command execution, agent lifecycle
- `workspaces/workspace.ts` - Unified workspace wrapper composing environment + file operations
- **No durable runtime snapshot** - workspace instances live in memory only (`activeWorkspaces` map)

## Migration Goals
1. Preserve existing behaviour for local execution while enabling worker-backed execution via the new coordinator.
2. Introduce a persisted runtime snapshot so state survives process restarts and cross-worker coordination.
3. Isolate workspace orchestration from task lifecycle logic to simplify maintenance and testing.

## New Feature Structure

```
apps/backend/src/features/workspace/
├── api/
│   ├── workspace.routes.ts              # New unified workspace routes
│   └── __tests__/
├── services/
│   ├── workspace-coordinator.service.ts  # Main orchestrator
│   ├── workspace-materializer.service.ts # Local provisioning
│   ├── worker-provisioner.service.ts     # Worker provisioning
│   ├── preview-manager.service.ts        # Preview lifecycle
│   ├── agent-controller.service.ts       # Agent lifecycle
│   └── __tests__/
├── drivers/                              # Moved from apps/backend/src/environments/
│   ├── base-environment.ts               # Abstract environment interface
│   ├── local-environment.ts              # Local filesystem execution
│   ├── docker-environment.ts             # Docker container execution
│   ├── remote-worker-environment.ts      # NEW: Worker RPC client
│   └── __tests__/
├── core/                                 # Moved from apps/backend/src/workspaces/
│   ├── workspace.ts                      # Unified workspace wrapper
│   └── __tests__/
├── storage/
│   ├── workspace-runtime.storage.ts      # Runtime snapshot persistence
│   └── __tests__/
├── types/
│   ├── workspace-runtime.types.ts        # Runtime snapshot schema
│   ├── provision-plan.types.ts           # Provisioning plans
│   ├── workspace-handle.types.ts         # Workspace references
│   └── environment.types.ts              # Environment interfaces
├── utils/
│   ├── git.utils.ts                      # Git operations (from utils/)
│   ├── workspace.utils.ts                # Workspace helpers
│   └── __tests__/
└── index.ts                              # Feature exports
```

## Phase Breakdown

### Phase 0 – Feature Skeleton & Storage (Session 1)
**Goal**: Set up new feature structure and runtime snapshot persistence

- [ ] Create `features/workspace/` directory structure
- [ ] Define `WorkspaceRuntimeSnapshot` schema in `types/workspace-runtime.types.ts`
  - Include: taskId, repositoryId, configRevision, executionTarget, agent state, preview state, locks
  - Add Zod validation schemas
- [ ] Implement `storage/workspace-runtime.storage.ts`
  - CRUD operations: `save()`, `load()`, `update()`, `delete()`, `list()`
  - File-based storage at `data/workspace-runtime/{taskId}.json`
  - Optimistic locking via `configRevision` field
  - Inject `StorageBackend` via `@inject(StorageSymbol)`
- [ ] Register storage adapter in `container/container.ts`
- [ ] Unit tests for storage adapter

### Phase 1 – Move Environments (Session 2)
**Goal**: Relocate environment drivers into workspace feature

- [ ] Copy `environments/` → `features/workspace/drivers/`
  - `base-environment.ts` - Abstract environment interface
  - `local-environment.ts` - Local filesystem execution
  - `docker-environment.ts` - Docker container execution
- [ ] Copy `workspaces/workspace.ts` → `features/workspace/core/workspace.ts`
- [ ] Update all imports to point to new locations
- [ ] Add `@injectable()` decorators to environment classes
- [ ] Register environments in container
- [ ] Run tests to verify no breakage

### Phase 2 – Coordinator Skeleton (Session 3)
**Goal**: Create facade that delegates to existing WorkspaceService

- [ ] Implement `services/workspace-coordinator.service.ts`
  - Methods: `ensureWorkspace()`, `getStatus()`, `runCommand()`, `startPreview()`, `startAgent()`
  - Initially delegate ALL calls to existing `WorkspaceService`
  - Inject both `WorkspaceService` and `WorkspaceRuntimeStorage`
  - Add `@injectable()` decorator
- [ ] Create `api/workspace.routes.ts` (new routes calling coordinator)
- [ ] Register coordinator in container
- [ ] Integration tests verifying coordinator delegates correctly

### Phase 3 – Runtime-backed Provisioning (Session 4-5)
**Goal**: Persistent snapshots and declarative provisioning

- [ ] Define `types/provision-plan.types.ts`
  - `WorkspaceProvisionPlan` interface (mode, branch, paths, credentials)
- [ ] Extend `RepositoryService` with provisioning methods
  - `createProvisionPlan(taskId, repoConfig)` - Generate provisioning plan
  - `resolveCredentials(repoConfig)` - Bundle credentials for workers
- [ ] Implement `services/workspace-materializer.service.ts` (local)
  - `applyPlan()` to execute local provisioning
  - Reuse git utilities from existing codebase
  - Update runtime snapshot after provisioning
- [ ] Implement `services/worker-provisioner.service.ts` (remote)
  - `prepareWorkspace()` to delegate to worker
  - RPC calls to `worker-app.ts` endpoints
  - Update runtime snapshot with worker locator
- [ ] Update coordinator to use snapshot-driven logic
  - Check snapshot → provision if needed → return handle
  - Support both local and worker modes based on repo config
- [ ] Integration tests for provisioning flows

### Phase 4 – Preview & Agent Controllers (Session 6)
**Goal**: Extract preview and agent lifecycle management

- [ ] Implement `services/preview-manager.service.ts`
  - Methods: `start()`, `stop()`, `restart()`, `getStatus()`
  - Delegate to `LocalEnvironment` or `RemoteWorkerEnvironment` based on snapshot
  - Update snapshot during state transitions
- [ ] Implement `services/agent-controller.service.ts`
  - Methods: `startAgent()`, `stopAgent()`, `getStatus()`
  - Claim/release agent slot in snapshot using CAS
  - Stream events through coordinator
- [ ] Create `drivers/remote-worker-environment.ts`
  - Proxy `runCommand`, `startPreview`, `runAgent` to worker
  - Implement same interface as `LocalEnvironment`
- [ ] Update coordinator to delegate to managers
- [ ] Integration tests for preview/agent lifecycle

### Phase 5 – Migration (Session 7-8)
**Goal**: Switch existing code to use new feature

- [ ] Update `TaskService`
  - Replace `WorkspaceService` injection with `WorkspaceCoordinator`
  - Update method calls to use coordinator API
  - Subscribe to coordinator events instead of workspace events
- [ ] Update `AgentService`
  - Use `AgentController` instead of direct environment calls
  - Update event handling
- [ ] Update routes
  - Switch all routes to use new workspace feature routes
  - Remove old route handlers
- [ ] Regression testing
  - Run full test suite
  - Verify no behavior changes
  - Test mixed scenarios (local + worker)

### Phase 6 – Cleanup (Session 9)
**Goal**: Remove legacy code

- [ ] Delete legacy services
  - Remove `services/workspace.service.ts` (1191 lines)
  - Remove old environment coupling in TaskService
  - Remove `environments/` directory (now in `features/workspace/drivers/`)
  - Remove `workspaces/` directory (now in `features/workspace/core/`)
- [ ] Delete legacy caches
  - Remove `activeWorkspaces` map
  - Remove `backgroundTasks` map
- [ ] Documentation
  - Update architecture docs
  - Add `features/workspace/README.md`
  - Document migration guide
- [ ] Final validation
  - All tests passing
  - Performance benchmarks
  - Staging deployment

## Testing Strategy
- Unit tests for `WorkspaceRuntimeStore` CAS semantics and snapshot validation.
- Contract tests for `WorkspaceProvisionPlan` generation given various repo configs.
- Integration tests that run `TaskService` flows under both local and worker-backed modes, ensuring agents/preview/logs behave the same.
- Chaos tests that restart the backend mid-operation to confirm runtime snapshots restore state.

## Risks & Mitigations
| Risk | Mitigation |
|------|------------|
| Snapshot divergence vs actual worker state | Add reconciliation jobs that ping workers and reconcile snapshot discrepancies; emit alerts on repeated mismatches. |
| Feature flag drift | Keep coordinator facade in place until both `TaskService` and routes no longer depend on `WorkspaceService`; enforce via TypeScript deprecations. |
| Worker API changes | Define typed clients/interfaces for worker RPCs and version them; add compatibility mode for older worker releases during rollout. |
| Performance regressions | Instrument coordinator methods (setup/preview/agent) with metrics before cutover; compare to baseline. |

## Dependencies
- Worker endpoints for workspace prep, command execution, preview control, file operations, and agent lifecycle must be stable.
- Repository service must expose the provisioning plan + credential bundle API before coordinator adopts it.
- Storage backend must support atomic updates (or we need optimistic locking implemented within the runtime store layer).

## Key Design Decisions

### 1. Clean Break Migration
- **No backward compatibility layer** - build new feature completely separate
- Existing `WorkspaceService` remains unchanged during implementation
- Migration happens in Phase 5 (one-time switch)
- Allows cleaner design without legacy constraints

### 2. Feature-First Organization
- All workspace code lives under `features/workspace/`
- Environments moved from `environments/` → `features/workspace/drivers/`
- Workspaces moved from `workspaces/` → `features/workspace/core/`
- Git utilities extracted to `features/workspace/utils/`

### 3. Storage Strategy
- File-based using existing `StorageBackend` interface
- Runtime snapshots at: `data/workspace-runtime/{taskId}.json`
- Optimistic locking via `configRevision` field
- No new infrastructure dependencies

### 4. Dependency Injection
- All services/drivers decorated with `@injectable()`
- Constructor injection for all dependencies
- Singleton scope for stateless services
- Container registration in `container/container.ts`

### 5. Unified Execution Interface
- Support both local and Docker workers from day one
- Single coordinator API for all execution modes
- Environment drivers implement common `BaseEnvironment` interface
- Mode selection based on repository config

## Success Criteria

✅ **Phase 0**: Runtime snapshots persist and load correctly
✅ **Phase 1**: Environments relocated without breaking existing functionality
✅ **Phase 2**: Coordinator delegates to existing service without regressions
✅ **Phase 3**: Workspaces provision via declarative plans and survive process restarts
✅ **Phase 4**: Preview and agent operations update snapshots atomically
✅ **Phase 5**: All existing tests pass with new implementation
✅ **Phase 6**: Legacy code removed, all tests green, production ready

## Rollout Plan

### Development Phase (Phases 0-4)
1. Build new feature in isolation under `features/workspace/`
2. Keep existing `WorkspaceService` and routes functioning normally
3. Write comprehensive unit and integration tests for new feature
4. No changes to existing consumers (TaskService, AgentService, routes)

### Migration Phase (Phase 5)
1. Update TaskService to inject `WorkspaceCoordinator` instead of `WorkspaceService`
2. Update AgentService to use `AgentController`
3. Switch routes to use new `api/workspace.routes.ts`
4. Run full regression test suite
5. Deploy to staging and validate

### Cleanup Phase (Phase 6)
1. Delete `services/workspace.service.ts` and related legacy code
2. Delete `environments/` and `workspaces/` directories (now in feature)
3. Update documentation
4. Deploy to production
5. Monitor metrics and validate performance

## Estimated Timeline
- **Phase 0**: 4-6 hours (feature skeleton + storage)
- **Phase 1**: 3-4 hours (move environments + workspaces)
- **Phase 2**: 3-4 hours (coordinator skeleton)
- **Phase 3**: 8-10 hours (provisioning logic)
- **Phase 4**: 6-8 hours (preview + agent controllers)
- **Phase 5**: 6-8 hours (migration)
- **Phase 6**: 2-3 hours (cleanup)

**Total**: ~33-43 hours over 9 focused sessions

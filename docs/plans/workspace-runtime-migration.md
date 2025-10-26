# Workspace Runtime Migration Plan

## Current Status (2025-10-26)

**Progress**: 4.5 of 6 phases complete (75%)

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 0 | ✅ Complete | Feature skeleton & runtime snapshot storage |
| Phase 1 | ✅ Complete | Stateless environment drivers (NEW implementation) |
| Phase 2 | ⏭️ Skipped | Facade approach (skipped for clean implementation) |
| Phase 3 | ✅ Complete | Workspace provisioning with snapshots |
| Phase 4 | ✅ Complete | Preview & agent controllers with driver delegation |
| Phase 5 | 🔄 In Progress | Migration (TaskService ✅, routes ✅, other services pending) |
| Phase 6 | ⏳ Pending | Cleanup (delete legacy code) |

**Key Achievement**: Implemented architecture-compliant design with stateless drivers and orchestrating managers. TaskService and new API routes successfully migrated to use workspace feature.

**Next Steps**: Complete Phase 5 - Migrate remaining services (AgentService, ToolService, ScreenshotOrchestrationService) and remove legacy route handlers.

### Implementation Notes

**Key Architectural Decisions Made**:

1. **Environment Drivers Built from Scratch** (Phase 1)
   - Original plan: "Copy `environments/` → `features/workspace/drivers/`"
   - **Actual**: Implemented NEW stateless drivers following architecture document
   - **Rationale**: Legacy environments were stateful and coupled. New drivers are stateless with state in snapshots.
   - **Result**: Clean separation - managers orchestrate, drivers execute

2. **Skipped Facade Pattern** (Phase 2)
   - Original plan: Create coordinator that delegates to old `WorkspaceService`
   - **Actual**: Skipped and went directly to clean implementation
   - **Rationale**: Facade would couple to legacy code and violate new architecture
   - **Result**: Cleaner design, faster implementation

3. **Driver Delegation Pattern** (Phase 4)
   - Managers (`PreviewManager`, `AgentController`) delegate to `LocalEnvironmentDriver`
   - Drivers create `PreviewServer` and `ClaudeCodeAgent` instances
   - Managers focus on orchestration and snapshot updates
   - Follows architecture: "Execution Layer – drivers that run commands, manage previews, and stream logs"

**Files Created** (all under `apps/backend/src/features/workspace/`):
```
drivers/
├── base-environment-driver.ts       # Stateless driver interface
├── local-environment-driver.ts      # Local execution (PreviewServer, ClaudeCodeAgent)
└── remote-worker-environment.ts     # Worker RPC client (stub)

services/
├── workspace-coordinator.service.ts # Lifecycle orchestrator
├── workspace-materializer.service.ts # Workspace provisioning
├── preview-manager.service.ts       # Preview lifecycle
└── agent-controller.service.ts      # Agent lifecycle

storage/
└── workspace-runtime.storage.ts     # Snapshot persistence

types/
├── workspace-runtime.types.ts       # Runtime snapshot schema
├── provision-plan.types.ts          # Provisioning plans
└── workspace-handle.types.ts        # Workspace references

utils/
└── git.utils.ts                     # Git operations
```

---

## Migration Approach
**Build a new workspace feature from scratch** following `docs/patterns/backend-feature-architecture.md`, then migrate existing services to use it and delete legacy code. This is a clean-break migration with no backward compatibility layer. Progress is tracked in `docs/migrations/workspace-runtime-migration-progress.md`; this plan reflects what remains.

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
│   ├── workspace-coordinator.service.ts      # Lifecycle orchestrator
│   ├── execution-target-manager.service.ts # Chooses local/docker/k8s providers
│   ├── workspace-materializer.service.ts   # Local provisioning
│   ├── worker-provisioner.service.ts       # Worker RPC bootstrap
│   ├── preview-manager.service.ts          # Preview lifecycle
│   ├── agent-controller.service.ts         # Agent lifecycle
│   └── __tests__/
├── drivers/                                # Execution adapters
│   ├── base-environment.ts               # Abstract environment interface
│   ├── local-environment.ts              # Local filesystem execution
│   ├── remote-worker-environment.ts      # Worker RPC client (docker/k8s)
│   └── __tests__/
├── execution-targets/
│   ├── execution-target-manager.ts       # Shared orchestration helpers
│   ├── providers/
│   │   ├── local.provider.ts
│   │   ├── docker.provider.ts            # Spins up worker-app containers
│   │   ├── kubernetes.provider.ts        # (future) pods
│   │   └── __tests__/
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

### Phase 0 – Feature Skeleton & Storage ✅ COMPLETE
**Goal**: Set up new feature structure and runtime snapshot persistence
**Status**: ✅ Complete (2025-10-25)

- [x] Create `features/workspace/` directory structure
- [x] Define `WorkspaceRuntimeSnapshot` schema in `types/workspace-runtime.types.ts`
  - Include: taskId, repositoryId, configRevision, executionTarget, agent state, preview state, locks
  - Add Zod validation schemas
- [x] Implement `storage/workspace-runtime.storage.ts`
  - CRUD operations: `save()`, `load()`, `update()`, `delete()`, `list()`
  - File-based storage at `data/workspace-runtime/{taskId}.json`
  - Optimistic locking via `configRevision` field
  - Inject `StorageBackend` via `@inject(StorageSymbol)`
- [x] Register storage adapter in `container/container.ts`
- [x] Unit tests for storage adapter (11 tests passing)

### Phase 1 – Environment Drivers ✅ COMPLETE
**Goal**: Implement stateless environment drivers for execution
**Status**: ✅ Complete (2025-10-25)
**Note**: Implemented NEW drivers from scratch instead of copying legacy code

- [x] Implement `drivers/base-environment-driver.ts` - Stateless driver interface
  - `runCommand()`, `startPreview()`, `stopPreview()`, `createAgent()`, `readFile()`
  - Shared interface for local and remote execution
- [x] Implement `drivers/local-environment-driver.ts` - Local execution
  - Manages active preview servers via Map (keyed by taskId)
  - Creates ClaudeCodeAgent instances on demand
  - Executes commands via ShellCommand
  - Stateless - state lives in runtime snapshots
- [x] Implement `drivers/remote-worker-environment.ts` - Worker RPC client (stub)
  - Placeholder for future worker support
  - Will delegate to worker API when implemented
- [x] Register drivers in container with singleton scope
- [x] All linting passed (biome)
- [x] All type checking passed (tsc)

### Phase 2 – Coordinator Skeleton ⏭️ SKIPPED
**Goal**: Create facade that delegates to existing WorkspaceService
**Status**: ⏭️ Skipped - went directly to Phase 3 implementation
**Rationale**: Facade approach would couple to legacy code. Instead implemented new architecture directly in Phase 3.

### Phase 3 – Runtime-backed Provisioning ✅ COMPLETE
**Goal**: Persistent snapshots and declarative provisioning
**Status**: ✅ Complete (2025-10-25)

- [x] Define `types/provision-plan.types.ts`
  - `WorkspaceProvisionPlan` interface (mode: direct/worktree/clone, branch, paths)
  - `CredentialBundle` for worker support (future)
- [x] Implement `utils/git.utils.ts` - Git operations
  - `isGitRepository()`, `isGitWorktree()`, `branchExists()`
  - `createBranch()`, `createGitWorktree()`, `cloneRepository()`
  - `getCurrentBranch()`, `checkoutBranch()`
- [x] Implement `services/workspace-materializer.service.ts` (local)
  - `applyPlan()` to execute local provisioning
  - `applyDirectMode()` - use repository path directly
  - `applyWorktreeMode()` - create git worktrees
  - `applyCloneMode()` - clone repository to new location
  - `validateWorkspace()` - check workspace exists
- [x] Implement `services/workspace-coordinator.service.ts`
  - `ensureWorkspace()` - provision or reuse workspace
  - `createProvisionPlan()` - generate provisioning plans
  - `determineProvisionMode()` - choose direct/worktree/clone
  - Snapshot-driven logic: check snapshot → provision if needed → return handle
  - Smart path generation for worktrees and clones
- [x] Register services in container
- [x] All linting passed (biome)
- [x] All type checking passed (tsc)

**Note**: Worker provisioner (`worker-provisioner.service.ts`) will be implemented when worker support is added.

### Phase 4 – Preview & Agent Controllers ✅ COMPLETE
**Goal**: Extract preview and agent lifecycle management
**Status**: ✅ Complete (2025-10-25)

- [x] Implement `services/preview-manager.service.ts`
  - Methods: `start()`, `stop()`, `restart()`, `getStatus()`, `getLogs()`, `streamLogs()`, `proxyRequest()`
  - Calls `WorkspaceCoordinator.ensureWorkspace()` before every start/restart, then fetches current `RepositoryConfig` to build execution plan
  - **Delegates to `LocalEnvironmentDriver`** instead of creating PreviewServer directly
  - Updates snapshot during state transitions (starting → started → failed)
  - Focuses on orchestration, driver handles execution
- [x] Implement `services/agent-controller.service.ts`
  - Methods: `startAgent()`, `stopAgent()`, `getStatus()`, `cleanup()`
  - Calls `WorkspaceCoordinator.ensureWorkspace()` before spawning agents and claims/releases agent slot in snapshot atomically
  - **Delegates to `LocalEnvironmentDriver.createAgent()`** instead of creating agents directly
  - Streams events through async generator
  - Focuses on orchestration, driver handles execution
- [x] Update `WorkspaceCoordinator` to delegate to managers
  - Preview delegation: `startPreview()`, `stopPreview()`, `restartPreview()`
  - Agent delegation: `startAgent()`, `stopAgent()`, `getAgentStatus()`
  - Cleanup: `cleanup()` stops all resources
- [x] All linting passed (biome)
- [x] All type checking passed (tsc)

**Architecture Compliance**:
- ✅ Managers orchestrate and update snapshots
- ✅ Drivers execute operations (create servers, run agents)
- ✅ Separation of concerns: Managers don't know HOW, drivers don't know WHY
- ✅ Stateless drivers with state in runtime snapshots

### Phase 5 – Migration 🔄 IN PROGRESS
**Goal**: Switch existing code to use new feature
**Status**: 50% complete (TaskService + routes done, services pending)

- [x] **Update `TaskService`** ✅ COMPLETE
  - Replaced `WorkspaceService` injection with `WorkspaceCoordinator`, `PreviewManager`, `AgentController`, `FileGateway`
  - Updated method calls to use coordinator helpers + managers
  - File: `apps/backend/src/services/task.service.ts`
- [x] **Create and register new API routes** ✅ COMPLETE
  - Implemented `features/workspace/api/preview.routes.ts` - preview server operations
  - Implemented `features/workspace/api/agent.routes.ts` - agent operations
  - Routes registered in `apps/backend/src/routes/index.ts` (lines 50-51)
  - Preview/agent routes invoke `PreviewManager`/`AgentController` directly
- [ ] **Update `AgentService`** ⏳ PENDING
  - Still uses old `WorkspaceService` injection
  - Needs migration to use `AgentController` instead of direct environment calls
  - File: `apps/backend/src/services/agent.service.ts`
- [ ] **Update `ToolService`** ⏳ PENDING
  - Still uses old `WorkspaceService` for workspace path resolution
  - Needs migration to use `FileGateway` or `WorkspaceCoordinator`
  - File: `apps/backend/src/services/tool.service.ts`
- [ ] **Update `ScreenshotOrchestrationService`** ⏳ PENDING
  - Still uses old `WorkspaceService` for workspace management
  - File: `apps/backend/src/services/screenshot-orchestration.service.ts`
- [ ] **Remove legacy route handlers** ⏳ PENDING
  - `apps/backend/src/routes/workspace.ts` - File operations still needed, preview/agent can be removed
  - `apps/backend/src/routes/preview-environment.ts` - Can be fully removed (replaced by `features/workspace/api/preview.routes.ts`)
  - `apps/backend/src/routes/tasks.ts` - Some task routes may still use old workspace patterns
- [ ] **Regression testing** ⏳ PENDING
  - Run full test suite
  - Manual validation checklist: create workspace → run install → start/stop preview → start/stop agent
  - Validate new routes work correctly

> **Status (Oct 26, 2025):** TaskService successfully migrated and new API routes are live. Remaining work: migrate AgentService, ToolService, and ScreenshotOrchestrationService, then remove legacy route handlers.

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

### Phase 7 – Remote Worker Support (Session 10+)
**Goal**: Turn on Docker/Kubernetes execution targets once the local flow is stable**

- [ ] Implement `execution-targets/providers/docker.provider.ts` to run `worker-app.ts` in containers
- [ ] Implement `execution-targets/providers/kubernetes.provider.ts` (pods) and `RemoteWorkerEnvironment` RPC client
- [ ] Wire `PreviewManager`/`AgentController` to delegate to remote drivers based on snapshot
- [ ] Update runtime snapshot reconciliation to detect remote worker failures/heartbeats
- [ ] End-to-end tests for Docker worker flow (provision → preview → agent → teardown)

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

✅ **Phase 0**: Runtime snapshots persist and load correctly - **COMPLETE**
✅ **Phase 1**: Environment drivers implemented with stateless architecture - **COMPLETE**
⏭️ **Phase 2**: Skipped (facade approach abandoned for clean implementation)
✅ **Phase 3**: Workspaces provision via declarative plans and survive process restarts - **COMPLETE**
✅ **Phase 4**: Preview and agent operations update snapshots atomically - **COMPLETE**
⏳ **Phase 5**: All existing tests pass with new implementation - **PENDING**
⏳ **Phase 6**: Legacy code removed, all tests green, production ready - **PENDING**

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

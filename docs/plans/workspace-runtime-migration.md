# Workspace Runtime Migration Plan

## Scope
Transition the current workspace/task orchestration (implemented primarily in `apps/backend/src/services/workspace.service.ts` and `apps/backend/src/services/task.service.ts`) to the refactored runtime architecture that introduces `WorkspaceCoordinator`, `WorkspaceRuntimeStore`, and worker-aware environment drivers. Execute the migration following the patterns described in `docs/patterns/backend-feature-architecture.md` so each feature module owns its domain, ports/adapters are explicit, and cross-cutting concerns (logging, auth, storage) remain consistent.

## Current Responsibilities Snapshot
- `WorkspaceService` owns everything: repository resolution, workspace path provisioning (clone/worktree/direct), environment creation, command/preview execution, screenshot capture, and background task tracking.
- `TaskService` mixes task lifecycle management with workspace readiness (setup, preview restarts, screenshot orchestration) and event subscription.
- No durable runtime snapshot exists; workspace instances live in memory (`activeWorkspaces` map).

## Migration Goals
1. Preserve existing behaviour for local execution while enabling worker-backed execution via the new coordinator.
2. Introduce a persisted runtime snapshot so state survives process restarts and cross-worker coordination.
3. Isolate workspace orchestration from task lifecycle logic to simplify maintenance and testing.

## Phase Breakdown
### Phase 0 – Foundations
- [ ] Introduce `WorkspaceRuntimeStore` with schema defined in `docs/architecture/workspace-runtime-refactor.md` and implement storage adapters.
- [ ] Add feature flag plumbing (e.g., `WORKSPACE_RUNTIME_V2`) to gate new paths per task/environment.

### Phase 1 – Coordinator Skeleton
- [ ] Implement `WorkspaceCoordinator` with read-only delegation to existing `WorkspaceService` (e.g., `ensureWorkspace` simply calls `workspaceService.getOrCreateWorkspace`).
- [ ] Update `TaskService` / routes to call the coordinator facade instead of `WorkspaceService` directly while still relying on legacy behaviour.

### Phase 2 – Runtime-backed Provisioning
- [ ] Integrate `WorkspaceRuntimeStore` into the coordinator so `ensureWorkspace` reads/writes snapshots before delegating.
- [ ] Extract `WorkspaceProvisionPlan` factories into `apps/backend/src/features/repositories/services/repository.service.ts` so provisioning logic becomes declarative.
- [ ] Implement `WorkspaceMaterializer` (local) and `WorkerProvisioner` (for docker/k8s) that consume the plan and update snapshots.
- [ ] Replace `WorkspaceService.activeWorkspaces` usage with runtime snapshot lookups; deprecate direct reuse of in-memory instances.

### Phase 3 – Preview & Agent Controllers
- [ ] Extract preview lifecycle logic from `Workspace`/`WorkspaceService` into `PreviewManager`, delegating to local or worker environments based on snapshot.
- [ ] Create `AgentController` that claims/updates `snapshot.agent` state instead of letting `TaskService` track agent activity implicitly.
- [ ] Update `TaskService` to subscribe to `WorkspaceRuntimeStore` events (or coordinator callbacks) instead of polling `Workspace` objects.

### Phase 4 – Environment Driver Split
- [ ] Introduce `RemoteWorkerEnvironment` client that proxies `runCommand`, `startPreview`, `runAgent`, file operations, etc., to `worker-app.ts` endpoints.
- [ ] Slim down `LocalEnvironment` to implement the same interface and remove preview ownership (no embedded `PreviewServer` instances).
- [ ] Update `Workspace` wrapper (or replace it with thin handles) to surface only file helpers; execution routes always go through the coordinator + environment interfaces.

### Phase 5 – Remove Legacy Service Coupling
- [ ] Delete unused methods from `WorkspaceService` and keep it as a compatibility shim (or fold entirely into the coordinator).
- [ ] Simplify `TaskService` to focus on task metadata, iterations, and event handling. Workspace setup/preview management should now belong to the coordinator/preview manager.
- [ ] Update background jobs (screenshot capture, auto preview) to call coordinator + preview manager APIs.

### Phase 6 – Cleanup & Hardening
- [ ] Remove feature flag once parity is validated in staging.
- [ ] Delete legacy caches (`activeWorkspaces`, `backgroundTasks`).
- [ ] Add regression tests covering mixed local/worker scenarios, restart recovery via runtime snapshots, and concurrent agent/preview operations.

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

## Rollout Checklist
1. Land runtime store + coordinator facade behind feature flag.
2. Deploy worker environment client and ensure worker APIs are live.
3. Enable feature flag for internal tasks; monitor logs/metrics.
4. Incrementally migrate task cohorts; rollback by toggling flag if issues arise.
5. Remove legacy paths once adoption reaches 100%.

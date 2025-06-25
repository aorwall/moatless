# Moatless Refactoring Checklist

This checklist provides step-by-step tasks for refactoring the Moatless codebase. Each task includes verification steps to ensure completion.

## Phase 1: Storage Consolidation

### ☐ 1. Remove TaskStorageService
**Files to modify:**
- [ ] Delete `/server/services/task-storage.service.ts`
- [ ] Update `/server/services/index.ts` to remove TaskStorageService
- [ ] Update `/server/services/task.service.ts`:
  - [ ] Change constructor to accept `StorageBackend` instead of `TaskStorageService`
  - [ ] Replace all `taskStorageService.*` calls with direct `storage.*` calls
- [ ] Update `/server/services/workspace.service.ts`:
  - [ ] Change constructor to accept `StorageBackend` instead of `TaskStorageService`
  - [ ] Replace `taskStorageService.getTaskWithEnvironmentId()` with direct storage calls

**Verification:**
- [ ] All tests pass
- [ ] No references to TaskStorageService remain in codebase
- [ ] Task creation and retrieval still works

## Phase 2: Workspace State Management

### ☐ 2. Update Workspace State Model
**Files to modify:**
- [ ] Update `/shared/types/workspace.ts`:
  - [ ] Add `REPOSITORY_MISSING = 'repository_missing'` to WorkspaceStatus enum
- [ ] Update `/server/workspaces/local.ts`:
  - [ ] Add check for workspace directory existence in `getStatus()`
  - [ ] Return `REPOSITORY_MISSING` if directory doesn't exist
- [ ] Update `/server/workspaces/base.ts`:
  - [ ] Update interface to handle new state

**Verification:**
- [ ] Workspace status correctly returns `REPOSITORY_MISSING` when directory missing
- [ ] All operations properly check for this state
- [ ] Appropriate errors returned for invalid operations

### ☐ 3. Fix Duplicate Workspace Creation
**Files to modify:**
- [ ] Update `/server/services/workspace.service.ts`:
  - [ ] In `getOrCreateWorkspace()`, check if workspace directory exists
  - [ ] Only create new workspace if directory missing
  - [ ] Return existing workspace if directory exists
  - [ ] Remove redundant environment lookups

**Verification:**
- [ ] No duplicate workspaces created
- [ ] Existing workspaces properly reused
- [ ] Performance improved (fewer lookups)

## Phase 3: Agent Execution Simplification

### ☐ 4. Consolidate Agent Methods
**Files to modify:**
- [ ] Update `/server/workspaces/base.ts`:
  - [ ] Remove abstract methods: `triggerAgent()`, `start()`
  - [ ] Keep only necessary abstract methods
- [ ] Update `/server/workspaces/local.ts`:
  - [ ] Remove `triggerAgent()` method
  - [ ] Remove `start()` method
  - [ ] Remove complex promise-based session ID handling
- [ ] Update `/server/services/workspace.service.ts`:
  - [ ] Remove `triggerAgent()` method
  - [ ] Remove `startWorkspace()` method
  - [ ] Add new `runAgent()` method:
    ```typescript
    async runAgent(taskId: string, message: string, iteration: TaskIteration): Promise<void>
    ```

**Verification:**
- [ ] Only one method to run agents exists
- [ ] Session ID properly captured from first system event
- [ ] Agent execution flow simplified to 3 layers

### ☐ 5. Simplify Task Service
**Files to modify:**
- [ ] Update `/server/services/task.service.ts`:
  - [ ] Update `sendMessage()` to use `workspaceService.runAgent()`
  - [ ] Remove complex workspace start logic
  - [ ] Ensure proper error handling for critical vs non-critical errors

**Verification:**
- [ ] Message sending triggers agent correctly
- [ ] Errors properly saved to iteration
- [ ] Critical errors abort flow

## Phase 4: Error Handling

### ☐ 6. Create Error Utilities
**Files to create:**
- [ ] Create `/server/utils/errors.ts`:
  - [ ] Define `CriticalError` class
  - [ ] Define error handling utilities
  - [ ] Create standardized error logging functions

**Files to modify:**
- [ ] Update all services to use new error utilities
- [ ] Ensure consistent error handling patterns

**Verification:**
- [ ] All services use consistent error handling
- [ ] Critical errors properly identified and handled
- [ ] Non-critical errors only logged

### ☐ 7. Update Iteration Error Handling
**Files to modify:**
- [ ] Update `/shared/types/task.ts`:
  - [ ] Add error fields to TaskIteration if missing
- [ ] Update workspace and task services:
  - [ ] Save critical errors to iteration
  - [ ] Mark iteration as failed on critical errors

**Verification:**
- [ ] Errors appear in task iterations
- [ ] Failed iterations properly marked
- [ ] Client can see error details

## Phase 5: Event System Cleanup

### ☐ 8. Remove WebSocket Support
**Files to modify:**
- [ ] Update `/server/routes/tasks.ts`:
  - [ ] Remove WebSocket endpoint handler
  - [ ] Keep only SSE endpoint
- [ ] Update `/server/app.ts`:
  - [ ] Remove WebSocket plugin registration
- [ ] Remove WebSocket-related dependencies from `package.json`

**Verification:**
- [ ] No WebSocket code remains
- [ ] SSE still works for event streaming
- [ ] Client receives events correctly

### ☐ 9. Remove Non-Agent Events
**Files to modify:**
- [ ] Update `/server/services/task.service.ts`:
  - [ ] Remove workspace setup event emissions
  - [ ] Remove task creation events
- [ ] Update `/server/services/workspace.service.ts`:
  - [ ] Remove workspace event emissions
- [ ] Keep only agent event emissions in agent execution flow

**Verification:**
- [ ] Only agent events in event stream
- [ ] No system events emitted
- [ ] Event stream cleaner and focused

## Phase 6: API Endpoints

### ☐ 10. Implement File Operation Endpoints
**Files to modify:**
- [ ] Update `/server/routes/tasks.ts`:
  - [ ] Add `GET /:taskId/files` - list files
  - [ ] Add `GET /:taskId/files/*` - read file
  - [ ] Add `PUT /:taskId/files/*` - write file
  - [ ] Add `POST /:taskId/search-files` - search files

**Verification:**
- [ ] All file operations work through API
- [ ] Proper state checking before operations
- [ ] Error handling for invalid states

### ☐ 11. Implement Git Operations
**Files to modify:**
- [ ] Update `/server/routes/tasks.ts`:
  - [ ] Add `GET /:taskId/git-diff` endpoint
  - [ ] Return structured diff response

**Verification:**
- [ ] Git diff endpoint returns proper data
- [ ] Works in appropriate states only

### ☐ 12. Implement Command Execution
**Files to modify:**
- [ ] Update `/server/routes/tasks.ts`:
  - [ ] Add `POST /:taskId/execute` endpoint
  - [ ] Include timeout and environment support

**Verification:**
- [ ] Commands execute in workspace
- [ ] Only works in IDLE state
- [ ] Timeout properly enforced

### ☐ 13. Implement Server Management
**Files to modify:**
- [ ] Update `/server/routes/tasks.ts`:
  - [ ] Add `POST /:taskId/server/start`
  - [ ] Add `POST /:taskId/server/stop`
  - [ ] Add `GET /:taskId/server/status`
  - [ ] Add `GET /:taskId/server/logs`

**Verification:**
- [ ] Server starts and stops correctly
- [ ] Status accurately reported
- [ ] Logs properly retrieved

## Phase 7: Background Task Tracking

### ☐ 14. Track Setup in Workspace State
**Files to modify:**
- [ ] Update `/server/workspaces/local.ts`:
  - [ ] Track setup progress in state
  - [ ] Expose setup status through workspace status
- [ ] Update `/server/services/workspace.service.ts`:
  - [ ] Include setup status in workspace status response

**Verification:**
- [ ] Setup progress visible in workspace status
- [ ] Background task completion tracked
- [ ] No race conditions

## Final Verification

### ☐ Complete System Test
- [ ] Run full E2E test suite
- [ ] Create task and verify flow
- [ ] Test all workspace operations
- [ ] Verify error handling works
- [ ] Check event streaming works
- [ ] Ensure no regressions

### ☐ Code Quality Check
- [ ] No duplicate code remains
- [ ] All TypeScript types correct
- [ ] No unused imports or variables
- [ ] Documentation updated
- [ ] All tests passing

### ☐ Performance Verification
- [ ] Reduced layers improve response time
- [ ] No unnecessary storage calls
- [ ] Event stream not overloaded
- [ ] Memory usage stable

## Notes for Implementation

1. **Test After Each Phase**: Don't wait until the end to test
2. **Commit Frequently**: Make small, atomic commits
3. **Update Tests**: Modify tests as you change implementations
4. **Document Changes**: Update comments and documentation
5. **Check Dependencies**: Ensure package.json is updated when removing features

This checklist ensures systematic refactoring while maintaining system stability throughout the process.
# Event Handling and Query Invalidation Fixes

## Overview

This document outlines issues with the current event handling system and provides a detailed plan for fixes. The main problems are:

1. Environment status changes don't emit events
2. Frontend event handlers don't handle all event types
3. Query invalidations are incomplete

## Current Issues

### 1. Missing Environment Event Emissions

**Problem**: Environment status transitions (not_created → initializing → ready → error/cleaning) don't emit events.

**Impact**: Frontend doesn't receive real-time updates when environments are being set up or torn down.

**Locations**:
- `apps/backend/environments/local-environment.ts:40-44` - Status changes without events
- `apps/backend/environments/docker-environment.ts:40-42` - Status changes without events

### 2. Incomplete Event Handler Mapping

**Problem**: The `environment` event type exists in the schema but isn't handled in frontend.

**Current State** (`apps/frontend/src/features/events/utils/event-handlers.ts`):
```typescript
export const EVENT_INVALIDATION_MAP = {
  message: [queryKeys.tasks.messages],
  result: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff],
  error: [queryKeys.tasks.detail],
  workspace: [queryKeys.tasks.workspaceStatus, queryKeys.tasks.workspaceSetup, queryKeys.tasks.gitDiff],
  task_status: [queryKeys.tasks.detail],
  // Missing: environment
}
```

**Impact**: Environment events are received but don't trigger any query invalidations.

### 3. Limited Status Affecting Events

**Problem**: Only `error` and `task_status` events trigger task list refreshes.

**Current State** (`apps/frontend/src/features/events/utils/event-handlers.ts:4`):
```typescript
export const STATUS_AFFECTING_EVENTS = ['error', 'task_status'] as const
```

**Impact**: Task lists don't update when:
- Iterations complete (`result` event)
- Workspace status changes (`workspace` event)
- Environment status changes (`environment` event)

## Root Causes

1. **No EventService in Environments**: Environment classes don't have access to EventService to emit events
2. **Incomplete Event Mapping**: Frontend event handlers weren't updated when new event types were added
3. **Conservative Invalidation**: STATUS_AFFECTING_EVENTS was kept minimal to avoid excessive re-renders

## Proposed Fixes

### Fix 1: Add EventService to Environments

**Approach**: Pass EventService through dependency injection to environment classes.

**Changes Required**:

1. Update environment constructors to accept optional EventService
2. Add status change tracking and event emission
3. Update Workspace class to pass EventService when creating environments

**Environment Event Payload Structure**:
```typescript
{
  status: EnvironmentStatus,
  previousStatus: EnvironmentStatus,
  workspaceId: string,
  error?: string // If status is 'error'
}
```

### Fix 2: Complete Event Handler Mapping

**Add to EVENT_INVALIDATION_MAP**:
```typescript
environment: [queryKeys.tasks.workspaceStatus]
```

### Fix 3: Expand Status Affecting Events

**Update STATUS_AFFECTING_EVENTS**:
```typescript
export const STATUS_AFFECTING_EVENTS = ['error', 'task_status', 'result', 'workspace', 'environment'] as const
```

### Fix 4: Remove Task "Running" Status

**Problem**: Task status "running" is redundant since it can be derived from agent state.

**Changes Required**:
1. Remove all instances of setting task status to "running"
2. Derive running state from agent state in frontend
3. Update task lifecycle documentation to reflect this change

**Impact**: Simplifies state management and removes duplicate status tracking between task and agent levels.

## Implementation Todo List

### Backend Tasks

- [ ] **Task 1**: Update BaseEnvironment to support EventService
  - File: `apps/backend/environments/base-environment.ts`
  - Add optional EventService property
  - Add protected method `emitStatusChange(newStatus, previousStatus)`
  - Example implementation:
    ```typescript
    protected async emitStatusChange(newStatus: EnvironmentStatus, previousStatus: EnvironmentStatus) {
      if (this.eventService) {
        await this.eventService.emitEvent(this.workspaceId, {
          type: 'environment',
          subType: `environment:${newStatus}`,
          payload: {
            status: newStatus,
            previousStatus,
            workspaceId: this.workspaceId,
          }
        });
      }
    }
    ```

- [ ] **Task 2**: Update LocalEnvironment to emit events
  - File: `apps/backend/environments/local-environment.ts`
  - Add EventService to constructor
  - Track status changes in `setup()` method
  - Emit `environment` events when status changes
  - Key locations:
    - Line 40: `this.status = 'initializing'` → Add event emission
    - Line 43: `this.status = 'ready'` → Add event emission
    - Line 44: `this.status = 'error'` → Add event emission with error details

- [ ] **Task 3**: Update DockerEnvironment to emit events
  - File: `apps/backend/environments/docker-environment.ts`
  - Add EventService to constructor
  - Emit events in `setup()`, `cleanup()` methods
  - Track container status transitions
  - Key locations:
    - Line 40: `this.status = 'initializing'` → Add event emission
    - Line 42: `this.status = 'ready'` → Add event emission
    - Error handling in `setup()` → Add event emission

- [ ] **Task 4**: Update Workspace class to pass EventService
  - File: `apps/backend/workspaces/workspace.ts`
  - Modify constructor to accept EventService
  - Pass EventService when creating LocalEnvironment/DockerEnvironment
  - Update environment factory method around line 50-60

- [ ] **Task 5**: Update WorkspaceService to pass EventService
  - File: `apps/backend/services/workspace.service.ts`
  - Update `createWorkspace` method to pass EventService to Workspace
  - Update `getOrCreateWorkspace` to handle EventService
  - Ensure EventService is available via dependency injection

- [ ] **Task 5.1**: Remove Task "Running" Status
  - File: `apps/backend/services/agent.service.ts`
  - Remove line 126: `await this.taskService.updateTaskStatus(task.id, 'running')`
  - Remove any other instances of setting task status to "running"
  - Task status should only be 'open', 'closed', or 'error'

### Frontend Tasks

- [ ] **Task 6**: Update event handler mappings
  - File: `apps/frontend/src/features/events/utils/event-handlers.ts`
  - Add `environment` to EVENT_INVALIDATION_MAP
  - Update STATUS_AFFECTING_EVENTS to include `result`, `workspace`, `environment`

- [ ] **Task 7**: Verify query invalidations work correctly
  - Test that workspace status updates when environment events arrive
  - Test that task lists refresh on all status-affecting events

- [ ] **Task 7.1**: Update frontend to derive running state from agent
  - Remove any frontend logic that tracks task "running" status
  - Use agent state from workspace status to determine if task is executing
  - Update task status displays to show derived running state

### Testing Tasks

- [ ] **Task 8**: Add tests for environment event emissions
  - Create unit tests for LocalEnvironment event emissions
  - Create unit tests for DockerEnvironment event emissions

- [ ] **Task 9**: Add integration tests for event flow
  - Test complete flow from environment status change to UI update
  - Verify all query invalidations trigger correctly

## Priority Order

1. **High Priority**: Tasks 6 (frontend event handlers) - Quick fix with immediate impact
2. **Medium Priority**: Tasks 1-5 (backend event emissions) - More complex but necessary
3. **Low Priority**: Tasks 8-9 (testing) - Important but can be done after implementation

## Expected Outcomes

After implementing these fixes:

1. All status transitions will emit appropriate events
2. Frontend will receive real-time updates for all state changes
3. UI components will automatically refresh when relevant data changes
4. Task lists will update whenever any task-related status changes
5. Task "running" status eliminated - derived from agent state instead
6. Simplified state management with no duplicate status tracking

**Note**: The task lifecycle documentation in `docs/patterns/task-lifecycle.md` needs to be updated to reflect the removal of task "running" status and clarify that running state is derived from agent state.

## Event Flow After Fixes

```
Environment Status Change
    ↓
Emit 'environment' event (Backend)
    ↓
SSE delivers event to Frontend
    ↓
GlobalEventStream receives event
    ↓
Event handler maps to query keys
    ↓
React Query invalidates:
  - queryKeys.tasks.workspaceStatus(taskId)
  - queryKeys.tasks.lists() (if status-affecting)
    ↓
Components re-render with fresh data
```

## Implementation Examples

### Backend: Environment Event Emission
```typescript
// In LocalEnvironment.setup()
async setup(): Promise<void> {
  const previousStatus = this.status;
  this.status = 'initializing';
  await this.emitStatusChange(this.status, previousStatus);
  
  try {
    // ... setup logic ...
    
    const readyPreviousStatus = this.status;
    this.status = 'ready';
    await this.emitStatusChange(this.status, readyPreviousStatus);
  } catch (error) {
    const errorPreviousStatus = this.status;
    this.status = 'error';
    await this.emitStatusChange(this.status, errorPreviousStatus, error);
    throw error;
  }
}
```

### Frontend: Updated Event Handlers
```typescript
// event-handlers.ts
export const EVENT_INVALIDATION_MAP = {
  message: [queryKeys.tasks.messages],
  result: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff],
  error: [queryKeys.tasks.detail],
  workspace: [queryKeys.tasks.workspaceStatus, queryKeys.tasks.workspaceSetup, queryKeys.tasks.gitDiff],
  task_status: [queryKeys.tasks.detail],
  environment: [queryKeys.tasks.workspaceStatus], // NEW
  // ... other mappings
} as const

export const STATUS_AFFECTING_EVENTS = ['error', 'task_status', 'result', 'workspace', 'environment'] as const
```

## Verification Steps

1. **Environment Events in Console**:
   - Start a task and open browser console
   - Filter for `[GlobalEventStream]` logs
   - Verify you see: `Event: type=environment, subType=environment:initializing`
   - Confirm transitions: `not_created` → `initializing` → `ready`

2. **Workspace Status Badge Updates**:
   - Open task detail page
   - Watch workspace status badge during setup
   - Should update in real-time as environment initializes

3. **Task List Refreshes**:
   - Open task list in sidebar
   - Create/complete/fail a task in another tab
   - Verify list updates without manual refresh

4. **Query Invalidation Verification**:
   - Use React Query DevTools
   - Trigger environment status change
   - Verify `['tasks', taskId, 'workspace']` query is invalidated
   - Check that `['tasks', 'list']` is invalidated for status-affecting events

## Testing Checklist

- [ ] Environment events appear in browser console
- [ ] Workspace status updates in real-time
- [ ] Task lists refresh on status changes
- [ ] No duplicate event emissions
- [ ] Error states properly propagated
- [ ] Query invalidations trigger correctly
- [ ] No performance degradation from excessive invalidations

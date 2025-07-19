# Task Lifecycle and Events Plan

## Overview

This document consolidates improvements needed for task lifecycle management and event handling. The main issues being addressed are:

1. **Redundant State Tracking**: Task "running" status duplicates agent state information
2. **Missing Event Emissions**: Environment status changes don't emit events for real-time updates
3. **Incomplete Event Handling**: Frontend doesn't handle all event types properly
4. **Unclear State Dependencies**: No clear documentation of how component states depend on each other
5. **Inconsistent API Responses**: Workspace status information not included in task responses
6. **Incorrect Status Display**: Components using hardcoded colors and wrong state derivation

## State Management Changes

### Remove Task "Running" Status

**Problem**: The task "running" status is redundant because it duplicates information already available in the agent state.

**Solution**: 
- Remove all backend code that sets task status to "running"
- Derive running state from `agent.state === 'running'` in frontend
- Task states will be limited to: `open`, `closed`, `error`

### State Dependency Matrix

This matrix shows how component states depend on each other hierarchically:

| Task State | Workspace State | Environment State | Agent State | Notes |
|------------|-----------------|-------------------|-------------|-------|
| `closed` | N/A | N/A | N/A | All resources cleaned up, no workspace exists |
| `error` | Any | Any | `idle` | Task error can occur at any workspace/environment state |
| `open` | `missing` | N/A | N/A | Task exists but workspace not yet created |
| `open` | `invalid` | N/A | N/A | Workspace path exists but corrupted/not git repo |
| `open` | `exists` | `not_created` | N/A | Workspace valid, environment not yet initialized |
| `open` | `exists` | `initializing` | N/A | Environment setup in progress |
| `open` | `exists` | `error` | N/A | Environment setup failed |
| `open` | `exists` | `cleaning` | N/A | Environment teardown in progress |
| `open` | `exists` | `ready` | `idle` | Fully operational, ready for agent execution |
| `open` | `exists` | `ready` | `running` | Agent actively executing iteration |

### Key Dependencies

1. **Task → Workspace**: 
   - `closed` task = no workspace exists
   - `open`/`error` task = workspace may exist in any state

2. **Workspace → Environment**:
   - `missing`/`invalid` workspace = no environment
   - `exists` workspace = environment can be in any state

3. **Environment → Agent**:
   - Only `ready` environment can have agent states
   - Other environment states = no agent state

### Workspace Status Response Strategy

Use a **hybrid approach** where components only appear in the response when they conceptually exist:

```typescript
// Closed task - minimal response
{ 
  task: { status: 'closed' } 
}

// Open task, workspace missing
{
  task: { status: 'open' },
  workspace: { state: 'missing', path: '/expected/path' }
  // No environment or agent fields
}

// Workspace exists, environment initializing
{
  task: { status: 'open' },
  workspace: { state: 'exists', path: '/actual/path', changeStatus: 'no_changes' },
  environment: { type: 'docker', status: 'initializing', createdAt: '2024-01-01T00:00:00Z' }
  // No agent field until environment is ready
}

// Everything ready
{
  task: { status: 'open' },
  workspace: { state: 'exists', ... },
  environment: { status: 'ready', ... },
  agent: { state: 'idle' },
  gitStatus: { ... },
  capabilities: { ... }
}
```

## Event System Improvements

### Add Environment Event Emissions

**Problem**: Environment status changes (not_created → initializing → ready → error/cleaning) don't emit events.

**Solution**: Pass EventService to environment classes and emit events on status changes.

**Event Payload Structure**:
```typescript
{
  type: 'environment',
  subType: `environment:${status}`,
  payload: {
    status: EnvironmentStatus,
    previousStatus: EnvironmentStatus,
    workspaceId: string,
    error?: string // If status is 'error'
  }
}
```

### Add Agent State Events

**Problem**: No dedicated agent state tracking - frontend incorrectly derives running state from iteration status.

**Solution**: Add 'agent' event type to track agent lifecycle (starting, running, completed, failed).

**Agent Event Structure**:
```typescript
{
  type: 'agent',
  subType: state, // 'starting', 'running', 'completed', 'failed'
  payload: {
    state: 'starting' | 'running' | 'completed' | 'failed',
    error?: string, // If state is 'failed'
    sessionId?: string // Agent session ID
  }
}
```

**Implementation Approach**:
- Agents yield their own state events
- AgentService handles event emission to EventService
- Workspace tracks running agent via setRunningAgent()
- Frontend derives running state from agent.state

### Complete Event Handler Mapping

**Add to EVENT_INVALIDATION_MAP**:
```typescript
environment: [queryKeys.tasks.detail], // Since workspace status is now in task response
agent: [queryKeys.tasks.detail]
```

### Expand Status Affecting Events

**Update STATUS_AFFECTING_EVENTS**:
```typescript
export const STATUS_AFFECTING_EVENTS = ['error', 'task_status', 'result', 'workspace', 'environment', 'agent'] as const
```

This ensures task lists refresh when any task-related status changes occur.

## API Changes

### Update TaskResponseSchema

Add optional workspace status fields that follow the dependency hierarchy:

```typescript
// Partial schemas for different states
const WorkspaceInfoSchema = z.object({
  state: WorkspaceExistenceSchema,
  path: z.string().optional(),
  changeStatus: WorkspaceChangeStatusSchema.optional(), // Only when state is 'exists'
})

const EnvironmentInfoSchema = z.object({
  type: z.enum(['local', 'docker']),
  status: EnvironmentStatusSchema,
  error: z.string().optional(),
  createdAt: z.string(),
})

const AgentInfoSchema = z.object({
  state: AgentStateSchema,
  type: z.string().optional(),
})

// Updated TaskResponseSchema
TaskResponseSchema = z.object({
  // ... existing fields ...
  workspace: WorkspaceInfoSchema.optional(), // Only present if task is not closed
  environment: EnvironmentInfoSchema.optional(), // Only present if workspace exists
  agent: AgentInfoSchema.optional(), // Only present if environment is ready
  gitStatus: GitStatusSchema.optional(), // Only when workspace exists
  capabilities: z.object({
    fileOperations: z.boolean(),
    commandExecution: z.boolean(),
    serverManagement: z.boolean(),
  }).optional(), // Only when environment is ready
})
```

### Response Examples

**Closed Task**:
```json
{
  "taskId": "123",
  "status": "closed",
  "iterations": []
  // No workspace, environment, or agent data
}
```

**Open Task with Missing Workspace**:
```json
{
  "taskId": "123",
  "status": "open",
  "iterations": [...],
  "workspace": {
    "state": "missing",
    "path": "/expected/workspace/path"
  }
  // No environment or agent data
}
```

**Task with Initializing Environment**:
```json
{
  "taskId": "123",
  "status": "open",
  "iterations": [...],
  "workspace": {
    "state": "exists",
    "path": "/actual/workspace/path",
    "changeStatus": "no_changes"
  },
  "environment": {
    "type": "docker",
    "status": "initializing",
    "createdAt": "2024-01-01T00:00:00Z"
  }
  // No agent data yet
}
```

**Fully Ready Task**:
```json
{
  "taskId": "123",
  "status": "open",
  "iterations": [...],
  "workspace": {
    "state": "exists",
    "path": "/actual/workspace/path",
    "changeStatus": "uncommitted"
  },
  "environment": {
    "type": "docker",
    "status": "ready",
    "createdAt": "2024-01-01T00:00:00Z"
  },
  "agent": {
    "state": "idle"
  },
  "gitStatus": { ... },
  "capabilities": {
    "fileOperations": true,
    "commandExecution": true,
    "serverManagement": true
  }
}
```

## Implementation Priority

1. **High Priority**:
   - Frontend event handler fixes (quick wins)
   - Remove task "running" status
   - Update TaskResponseSchema

2. **Medium Priority**:
   - Add EventService to environments
   - Implement environment event emissions
   - Update frontend to handle new response structure

3. **Low Priority**:
   - Add comprehensive tests
   - Update documentation

## Status Display and Icons

### Current Problems
1. **TaskCard** checks `task.status === 'running'` instead of deriving from agent state
2. **task-status-icon.tsx** uses hardcoded colors that violate the styling guide (text-red-500, text-yellow-500, text-green-500)
3. **TaskDataProvider** incorrectly derives `isRunning` from iteration status instead of agent state
4. **Inconsistent status determination** across components

### Status Icon Mapping (Styling Guide Compliant)

```typescript
// Task Status Icons with semantic color tokens
const statusConfig = {
  open: {
    icon: <PlayCircle className="h-3.5 w-3.5" />,
    color: 'text-status-info',  // NOT text-blue-500
    tooltip: 'Open'
  },
  running: {
    icon: <Loader2 className="h-3.5 w-3.5 animate-spin" />,
    color: 'text-status-warning',  // NOT text-yellow-500
    tooltip: 'Running'
  },
  error: {
    icon: <AlertCircle className="h-3.5 w-3.5" />,
    color: 'text-status-error',  // NOT text-red-500
    tooltip: 'Error'
  },
  closed: {
    icon: <CircleDot className="h-3.5 w-3.5" />,
    color: 'text-status-pending',  // NOT text-gray-500
    tooltip: 'Closed'
  },
  merged: {
    icon: <GitMerge className="h-3.5 w-3.5" />,
    color: 'text-status-success',  // NOT text-green-500
    tooltip: 'Merged'
  }
}

// Git Status Colors for Open Tasks
const gitStatusColors = {
  conflicts: 'text-status-error',      // NOT text-red-500
  hasChanges: 'text-status-warning',   // NOT text-yellow-500
  clean: 'text-status-success',        // NOT text-green-500
}
```

### Deriving Running State

```typescript
// Utility function to derive running state
export function getTaskRunningState(task: Task, workspaceStatus?: WorkspaceStatusResponse): boolean {
  return workspaceStatus?.agent?.state === 'running'
}

// Get display status (including derived running state)
export function getTaskDisplayStatus(task: Task, workspaceStatus?: WorkspaceStatusResponse): string {
  const isRunning = getTaskRunningState(task, workspaceStatus)
  return isRunning ? 'running' : task.status
}

// Complete status display logic
export function getTaskStatusDisplay(task: Task, workspaceStatus?: WorkspaceStatusResponse) {
  const isRunning = workspaceStatus?.agent?.state === 'running'
  
  // Running takes precedence
  if (isRunning) {
    return {
      icon: <Loader2 className="h-3.5 w-3.5 animate-spin" />,
      color: 'text-status-warning',
      tooltip: 'Running'
    }
  }
  
  // For open tasks with work done, show git status
  if (task.status === 'open' && task.iterations?.length > 0) {
    const lastIteration = task.iterations[task.iterations.length - 1]
    const gitStatus = lastIteration?.gitStatus
    
    if (gitStatus) {
      return {
        icon: <GitBranch className="h-3.5 w-3.5" />,
        color: gitStatus.hasConflicts ? 'text-status-error' :
               (gitStatus.commitsAhead > 0 || gitStatus.hasUncommittedChanges) ? 'text-status-warning' :
               'text-status-success',
        tooltip: gitStatus.hasConflicts ? 'Has conflicts' :
                 gitStatus.hasUncommittedChanges ? 'Uncommitted changes' :
                 gitStatus.commitsAhead > 0 ? `${gitStatus.commitsAhead} commits ahead` :
                 'Clean'
      }
    }
  }
  
  // Default status mapping
  return statusConfig[task.status] || statusConfig.open
}
```

### Component Updates Needed

1. **task-status-icon.tsx**: Remove all hardcoded colors (text-red-500, text-yellow-500, text-green-500)
2. **TaskCard**: Add workspace status query, replace `task.status === 'running'` checks
3. **TaskDataProvider**: Fix `isRunning` to use `workspaceStatus?.agent?.state === 'running'`
4. **Create TaskStatusBadge**: New component using semantic tokens and proper derivation

## Testing Strategy

### Unit Tests
- Test environment event emissions
- Test state dependency logic
- Test response structure based on state
- Test status derivation from agent state
- Test color token usage (no hardcoded colors)

### Integration Tests
- Test event flow from backend to frontend
- Test query invalidations
- Test UI updates based on events
- Test status display updates with workspace changes

### Manual Testing
- Verify environment status transitions emit events
- Verify task lists refresh on all status changes
- Verify workspace status badges update in real-time
- Verify no duplicate event emissions
- Verify correct icon and color display for all states
- Test theme switching (light/dark) with semantic tokens

## Implementation Progress Summary 

### Backend Implementation ✅ COMPLETED (Commit: 5e078f2d)
All backend tasks have been completed successfully. Here's what was implemented:

### Frontend Implementation ✅ MOSTLY COMPLETED 
Most frontend tasks are complete with only minor cleanup remaining:
- ✅ Event handlers updated to include environment events  
- ✅ Task status display now derives running state from agent.state
- ✅ TaskDataProvider properly handles new response structure
- ✅ Components use semantic color tokens for most cases
- ⚠️ **Remaining**: Fix hardcoded colors in task-status-icon.tsx git status display
- ⚠️ **Remaining**: Add loading states for missing workspace data

## Backend Implementation Summary ✅

All backend tasks have been completed successfully. Here's what was implemented:

### 1. Task "Running" Status Removal
- **Removed** task status transitions to/from 'running' in `agent.service.ts`
- **Updated** repository service to only check for 'open' status 
- **Fixed** command conditions to remove running status checks
- **Updated** API routes to exclude 'running' from valid status enum
- **Result**: Task status now only uses `open`, `closed`, `error` as intended

### 2. Environment Event System
- **Added** optional EventService parameter to BaseEnvironment constructor
- **Implemented** `emitStatusChange()` method with proper event structure
- **Updated** LocalEnvironment to emit events on all status transitions:
  - `not_created` → `initializing` → `ready` or `error`
- **Updated** DockerEnvironment to emit events during setup and cleanup:
  - Status transitions: `initializing`, `ready`, `error`, `cleaning`
- **Result**: All environment status changes now emit events for real-time frontend updates

### 3. EventService Dependency Chain
- **Modified** WorkspaceService to accept and pass EventService to environments
- **Updated** service container to provide EventService in correct dependency order
- **Fixed** all test files to include mock EventService instances
- **Result**: EventService properly flows through: ServiceContainer → WorkspaceService → Environment

### 4. Enhanced Task Response Schema
- **Created** new schemas: `WorkspaceInfoSchema`, `EnvironmentInfoSchema`, `AgentInfoSchema`
- **Enhanced** TaskResponseSchema with optional workspace status fields following dependency hierarchy:
  - `workspace` - Only present if task is not closed
  - `environment` - Only present if workspace exists  
  - `agent` - Only present if environment is ready
  - `gitStatus` - Only when workspace exists
  - `capabilities` - Only when environment is ready
- **Added** proper TypeScript type exports
- **Result**: API responses now provide structured workspace status information

### Technical Implementation Details
- All environment status transitions emit structured events with `type: 'environment'`
- Events include previous status, current status, and error details when applicable
- EventService integration maintains backward compatibility with optional parameter
- Schema changes are additive - existing API consumers won't break
- Test coverage maintained with proper mocking

## TODO Checklist

### Backend Tasks ✅ COMPLETED

- [x] **Remove Task "Running" Status**
  - [x] Remove `updateTaskStatus(task.id, 'running')` from `apps/backend/services/agent.service.ts:131` (lines 129-150)
  - [x] Remove reference in `repository.service.ts:127` that checked for both 'open' and 'running'
  - [x] Remove running status check from `command-conditions.ts:25`
  - [x] Remove 'running' from API route enum in `routes/tasks.ts:232`
  - [x] Ensure task status only uses: `open`, `closed`, `error`

- [x] **Add EventService to Environments**
  - [x] Update `BaseEnvironment` to accept optional EventService
  - [x] Add `emitStatusChange()` method to BaseEnvironment
  - [x] Update `LocalEnvironment` constructor and emit events in `setup()`:
    - [x] Emit 'initializing' event when setup starts
    - [x] Emit 'ready' event when setup completes in background commands
    - [x] Emit 'error' event when setup fails with error details
  - [x] Update `DockerEnvironment` constructor and emit events in `setup()` and `cleanup()`:
    - [x] Emit 'initializing' event when setup starts
    - [x] Emit 'ready' event when container becomes ready
    - [x] Emit 'error' event when setup fails with error details
    - [x] Emit 'cleaning' event when cleanup starts

- [x] **Pass EventService Through Dependency Chain**
  - [x] Update `WorkspaceService` constructor to accept EventService
  - [x] Pass EventService when creating environments (both LocalEnvironment and DockerEnvironment)
  - [x] Update `createWorkspaceService` function to accept EventService parameter
  - [x] Update `service-container.ts` to pass EventService to WorkspaceService
  - [x] Fix test files to include mock EventService

- [x] **Update Task Response Schema**
  - [x] Create `WorkspaceInfoSchema`, `EnvironmentInfoSchema`, `AgentInfoSchema` in `task.schemas.ts`
  - [x] Add optional workspace status fields to `TaskResponseSchema` following dependency hierarchy
  - [x] Update exported TypeScript types

### Frontend Tasks ✅ COMPLETED

- [x] **Update Event Handlers**
  - [x] Add `environment: [queryKeys.tasks.detail]` to `EVENT_INVALIDATION_MAP` in `event-handlers.ts`
  - [x] Add `agent: [queryKeys.tasks.detail]` to `EVENT_INVALIDATION_MAP`
  - [x] Update `STATUS_AFFECTING_EVENTS` to include `environment`

- [x] **Update Task Status Display**
  - [x] Remove any logic that tracks task "running" status
  - [x] Derive running state from `agent.state === 'running'`
  - [x] Update status badges/indicators to use derived state
  - [x] Fix task-status-icon.tsx to use semantic color tokens (text-status-*)
  - [x] Update TaskCard to use workspace status for running state  
  - [x] Fix TaskDataProvider isRunning derivation

- [ ] **Handle New Response Structure**
  - [x] Update components to handle optional workspace/environment/agent fields
  - [x] Update TypeScript types to match new response structure
  - [ ] Fix remaining hardcoded colors in task-status-icon.tsx (still uses text-red-500, text-yellow-500, text-green-500)
  - [ ] Add appropriate loading states for missing data

### Schema/Type Updates ✅ COMPLETED

- [x] **Update Task Response Schema**
  - [x] Create `WorkspaceInfoSchema`, `EnvironmentInfoSchema`, `AgentInfoSchema`
  - [x] Add optional fields to `TaskResponseSchema`
  - [x] Update exported types

- [x] **Update Frontend Types**
  - [x] Update task response types to match new schema
  - [x] Update any type guards or validation functions

### Documentation Updates

- [ ] **Update Task Lifecycle Documentation**
  - [ ] Remove references to task "running" status
  - [ ] Add state dependency matrix
  - [ ] Update state transition descriptions

- [ ] **Update API Documentation**
  - [ ] Document new response structure
  - [ ] Add examples for different states
  - [ ] Document when fields are present/absent

### Testing Tasks

- [ ] **Backend Tests**
  - [ ] Add tests for environment event emissions
  - [ ] Add tests for state-dependent response structure
  - [ ] Update existing tests that expect "running" status

- [ ] **Frontend Tests**
  - [ ] Add tests for derived running state
  - [ ] Add tests for handling optional response fields
  - [ ] Update event handler tests

- [ ] **Integration Tests**
  - [ ] Test complete event flow from environment to UI
  - [ ] Test query invalidations for all event types
  - [ ] Test progressive disclosure of workspace status

### Verification Steps

- [ ] **Event Verification**
  - [ ] Environment events appear in browser console
  - [ ] Events have correct type and payload structure
  - [ ] No duplicate event emissions

- [ ] **UI Updates**
  - [ ] Workspace status badges update in real-time
  - [ ] Task lists refresh on all status changes
  - [ ] Running state correctly derived from agent state

- [ ] **API Response Verification**
  - [ ] Closed tasks return minimal data
  - [ ] Missing workspaces don't include environment data
  - [ ] Environments not ready don't include agent data
  - [ ] All fields present when system is fully ready

- [ ] **Performance**
  - [ ] No excessive re-renders from event handling
  - [ ] Query invalidations are targeted, not global
  - [ ] Response payload sizes are appropriate

## Concrete Implementation Patterns

### Environment Event Emission Pattern

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

### Event Flow Sequence

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
  - queryKeys.tasks.detail(taskId) // Since workspace status is in task response
  - queryKeys.tasks.lists() (if status-affecting)
    ↓
Components re-render with fresh data
```

## Success Criteria

1. **Simplified State Management**: No duplicate state tracking between task and agent
2. **Complete Event Coverage**: All state transitions emit appropriate events
3. **Clear Dependencies**: API responses reflect actual component dependencies
4. **Real-time Updates**: UI components update automatically on state changes
5. **Type Safety**: Frontend and backend types align with new structure
6. **No Regressions**: Existing functionality continues to work correctly
7. **Styling Guide Compliance**: All status colors use semantic tokens (text-status-*)
8. **Consistent Status Display**: All components derive running state from agent state
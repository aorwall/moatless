# State Simplification Implementation Instructions

## Overview

This document provides instructions for an LLM to implement state simplification in the Moatless Swarm UI codebase. The goal is to separate concerns and create a clearer state model where each entity tracks only its own state.

## Current Problems

1. **Overlapping State Responsibilities**
   - Task status includes execution states (`running`, `error`) that should belong to iterations
   - Workspace status is a composite of environment state and agent state
   - Agent running state is tracked in multiple places

2. **Confusing Terminology**
   - `WorkspaceStatus` vs `WorkspaceState` both exist with different meanings
   - Environment "ready" state vs workspace "idle" state
   - Task "error" vs iteration "failed" vs iteration "error"

3. **Composite States**
   - Workspace status mixes file system state, environment state, and agent state
   - Task status reflects iteration execution state rather than container state

## Proposed State Model

### 1. Task States (Container Level)

```typescript
type TaskStatus = 'open' | 'closed'
```

- **open**: Active task container, can receive new iterations
- **closed**: Archived task, no new iterations allowed, resources cleaned up

### 2. Iteration States (Execution Level)

```typescript
type IterationStatus = 'created' | 'active' | 'completed' | 'failed'
```

- **created**: Awaiting execution
- **active**: Currently executing
- **completed**: Successfully finished
- **failed**: Execution error (combines current 'error' and 'failed')

### 3. Workspace States (File System Level)

```typescript
type WorkspaceState = 'exists' | 'missing' | 'invalid'
```

- **exists**: File system path exists and is valid
- **missing**: Path doesn't exist (needs creation)
- **invalid**: Path exists but corrupted/inaccessible

### 4. Workspace Change Status (Git Level)

```typescript
type WorkspaceChangeStatus = 'no_changes' | 'merged' | 'committed_unmerged' | 'uncommitted' | 'conflicts'
```

- **no_changes**: No changes compared to original branch
- **merged**: All changes on branch are merged to main
- **committed_unmerged**: Has committed changes not yet merged to main
- **uncommitted**: Has uncommitted changes in workspace
- **conflicts**: Has conflicts with the main branch

### 5. Environment States (Execution Environment Level)

```typescript
type EnvironmentStatus = 'not_created' | 'initializing' | 'ready' | 'cleaning' | 'error'
```

- **not_created**: Initial state
- **initializing**: Setup in progress
- **ready**: Operational (can execute commands/agents)
- **cleaning**: Teardown in progress
- **error**: Setup or runtime failure

### 6. Agent State (Within Environment)

```typescript
type AgentState = 'idle' | 'running'
```

- **idle**: No agent active
- **running**: Agent executing

## Implementation Instructions

### Step 1: Update Type Definitions

#### 1.1 Event Type Updates
**File**: `packages/types/src/schemas/event.schemas.ts`

Add the new environment event type:
```typescript
// Find EventTypeSchema and add 'environment' to the enum
export const EventTypeSchema = z.enum([
  'message',
  'result',
  'error',
  'workspace',
  'environment',  // Add this new event type
  'task_status',
  'start',
  'stop',
  'server',
  'tool',
  'completion',
])
```

#### 1.2 Task Schema Updates
**File**: `packages/types/src/schemas/task.schemas.ts`

Find line 114 where TaskResponseSchema defines status and change:
```typescript
status: z.enum(['open', 'running', 'error', 'merged', 'closed']),
```
To:
```typescript
status: z.enum(['open', 'closed']),
```

Find line 43 where TaskIterationSchema defines status and change:
```typescript
status: z.enum(['created', 'active', 'completed', 'error', 'failed']),
```
To:
```typescript
status: z.enum(['created', 'active', 'completed', 'failed']),
```

#### 1.2 Workspace Schema Updates
**File**: `packages/types/src/schemas/workspace.schemas.ts`

1. Find and remove the WorkspaceStatusSchema at line 9:
```typescript
export const WorkspaceStatusSchema = z.enum(['not_setup', 'setting_up', 'idle', 'running'])
```

2. Find WorkspaceStateSchema at line 13 and rename it to EnvironmentStatusSchema (keep the same values).

3. After the EnvironmentStatusSchema, add these new schemas:
```typescript
// Add after EnvironmentStatusSchema
export const WorkspaceExistenceSchema = z.enum(['exists', 'missing', 'invalid'])

export const WorkspaceChangeStatusSchema = z.enum(['no_changes', 'merged', 'committed_unmerged', 'uncommitted', 'conflicts'])

export const AgentStateSchema = z.enum(['idle', 'running'])
```

4. Update the derived types section (around line 150) to add:
```typescript
export type WorkspaceExistence = z.infer<typeof WorkspaceExistenceSchema>
export type WorkspaceChangeStatus = z.infer<typeof WorkspaceChangeStatusSchema>
export type AgentState = z.infer<typeof AgentStateSchema>
```

#### 1.3 Update WorkspaceStatusResponse
**File**: `packages/types/src/schemas/workspace.schemas.ts`

Find WorkspaceStatusResponseSchema at line 174 and replace the entire schema with:
```typescript
export const WorkspaceStatusResponseSchema = z.object({
  workspace: z.object({
    state: WorkspaceExistenceSchema,
    path: z.string().optional(),
    changeStatus: WorkspaceChangeStatusSchema,
  }),
  environment: z.object({
    type: z.enum(['local', 'docker']),
    status: EnvironmentStatusSchema,
    error: z.string().optional(),
    createdAt: z.string(),
  }),
  agent: z.object({
    state: AgentStateSchema,
    type: z.string().optional(),
  }),
  gitStatus: GitStatusSchema.optional(),
  capabilities: z.object({
    fileOperations: z.boolean(),
    commandExecution: z.boolean(),
    serverManagement: z.boolean(),
  }),
})
```

### Step 2: Backend Implementation

#### 2.1 Task Service Updates
**File**: `apps/backend/services/task.service.ts`

1. **In the `startTask` method (line 602)**: Remove the line that sets task status to 'running':
   ```typescript
   // DELETE THIS LINE:
   task.status = 'running'
   ```

2. **In the `handleResultEvent` method (line 219)**: Change the task status update to always be 'open':
   ```typescript
   // Find this line:
   const newStatus = event.isError || resultPayload.is_error ? 'error' : 'open'
   // Replace with:
   const newStatus = 'open'
   ```

3. **In the `handleErrorEvent` method (line 274)**: Remove the line that sets task status to 'error':
   ```typescript
   // DELETE THIS LINE:
   task.status = 'error'
   ```

4. **In the `sendMessage` method**:
   - Line 414: Delete `metadata.status = 'running'`
   - Line 998: Delete `metadata.status = 'running'`
   - Line 1034: Delete `metadata.status = 'error'`

5. **In the `getTask` method (lines 781-789)**: Remove the entire block that checks if agent is running:
   ```typescript
   // DELETE THIS ENTIRE BLOCK:
   if (task && task.status === 'running') {
     const isAgentRunning = await this.agentService.getTaskRunningStatus(taskId)
     if (!isAgentRunning) {
       task.status = 'open'
       await this.storage.saveTask(task)
     }
   }
   ```

#### 2.2 Workspace Service Updates
**File**: `apps/backend/services/workspace.service.ts`

In the `getWorkspaceStatus` method (line 566), update the return structure to use the new separated states:
```typescript
// The method should now delegate to the workspace to get the new status structure
return workspace.getWorkspaceStatus(backgroundTaskCount)
```

Note: The actual implementation of the new status structure will be in the Workspace class.

#### 2.3 Workspace Class Updates
**File**: `apps/backend/workspaces/workspace.ts`

1. **Remove the `getStatus()` method (lines 312-333)**:
   Delete the entire method that returns composite WorkspaceStatus.

2. **Add new state methods after line 311**:
```typescript
getWorkspaceExistence(): WorkspaceExistence {
  if (!this.workspacePath) return 'missing'
  
  // Use the existing isWorkspacePathValid method
  return this.isWorkspacePathValid().then(valid => valid ? 'exists' : 'invalid')
}

getEnvironmentStatus(): EnvironmentStatus {
  return this.environment.getStatus()
}

getAgentState(): AgentState {
  return this.environment.isRunning() ? 'running' : 'idle'
}
```

3. **Update the `getWorkspaceStatus` method (starting at line 427)** to return the new structure:
```typescript
async getWorkspaceStatus(backgroundTaskCount = 0): Promise<WorkspaceStatusResponse> {
  // Get git status
  const gitStatus = await this.getGitStatus()
  
  // Get workspace existence
  const workspaceExists = await this.isWorkspacePathValid()
  const workspaceState = workspaceExists ? 'exists' : 'missing'
  
  // Calculate change status
  const changeStatus = this.calculateChangeStatus(gitStatus)
  
  // Get environment and agent states
  const environment = this.getEnvironment()
  const environmentStatus = environment.getStatus()
  const agentState = environment.isRunning() ? 'running' : 'idle'
  
  // Determine capabilities
  const capabilities = {
    fileOperations: workspaceExists,
    commandExecution: environmentStatus === 'ready',
    serverManagement: environmentStatus === 'ready',
  }
  
  return {
    workspace: {
      state: workspaceState,
      path: this.workspacePath,
      changeStatus,
    },
    environment: {
      type: environment.getType(),
      status: environmentStatus,
      error: environment.getError(),
      createdAt: environment.getCreatedAt().toISOString(),
    },
    agent: {
      state: agentState,
      type: agentState === 'running' ? this.runningAgent?.getAgentType() : undefined,
    },
    gitStatus,
    capabilities,
  }
}

private calculateChangeStatus(gitStatus: GitStatus | null): WorkspaceChangeStatus {
  if (!gitStatus) return 'no_changes'
  
  // Check for conflicts
  if (gitStatus.conflicts > 0) return 'conflicts'
  
  // Check for uncommitted changes
  if (gitStatus.uncommittedChanges > 0) return 'uncommitted'
  
  // Check for committed but unmerged changes
  if (gitStatus.ahead > 0) return 'committed_unmerged'
  
  // Check if all changes are merged (ahead = 0, can be behind)
  if (gitStatus.ahead === 0 && gitStatus.uncommittedChanges === 0) {
    // If we're behind but have no local changes, consider it merged
    return 'merged'
  }
  
  // No changes
  return 'no_changes'
}
```

#### 2.4 Environment Event Emission
**File**: `apps/backend/workspaces/environment.ts` (or wherever environment status changes)

Add event emission when environment status changes:
```typescript
// When environment status changes, emit an event
private async setStatus(newStatus: EnvironmentStatus) {
  const previousStatus = this.status
  this.status = newStatus
  
  // Emit environment event
  await this.eventService.emitEvent({
    taskId: this.taskId,
    type: 'environment',
    subType: newStatus, // 'initializing', 'ready', 'cleaning', 'error', etc.
    payload: {
      previousStatus,
      status: newStatus,
      environmentType: this.getType(),
    },
    eventId: randomUUID(),
    timestamp: new Date(),
    isError: newStatus === 'error',
  })
}
```

#### 2.5 Agent Service Updates
**File**: `apps/backend/services/agent.service.ts`

1. **In the `processAgentExecution` method (around line 124)**, remove the task status update:
   ```typescript
   // DELETE THESE LINES:
   const task = await this.storage.getTask(taskId)
   if (task) {
     task.status = 'running'
     await this.storage.saveTask(task)
   }
   ```

2. **In the finally block of `processAgentExecution` (around line 136)**, remove the entire section that updates task status back to 'open':
   ```typescript
   // DELETE THIS ENTIRE BLOCK:
   const task = await this.storage.getTask(taskId)
   if (task && task.status === 'running') {
     const previousStatus = task.status
     task.status = 'open'
     await this.storage.saveTask(task)
     
     await this.eventService.emitEvent({
       taskId,
       type: 'task_status',
       subType: 'open',
       payload: {
         status: 'open',
         previousStatus,
       },
       eventId: randomUUID(),
       timestamp: new Date(),
       agentType: task.agentType,
       isError: false,
     })
   }
   ```

Note: Keep the `runningAgents` map as it's still used for validation to prevent multiple agents running on the same task.

### Step 3: Frontend Updates

Based on deep analysis of the frontend codebase, here are the comprehensive changes needed:

#### 3.1 Task Status Icon Component Updates

**File**: `apps/frontend/src/features/tasks/components/task-status-icon.tsx`

Update the task status configuration to handle the new simplified model:

```typescript
// Remove status configurations for 'running', 'error', 'merged'
// Keep only 'open' and 'closed'
// Add logic to show iteration status alongside task status

const getIterationStatus = (task: Task) => {
  if (!task.iterations || task.iterations.length === 0) return null
  const lastIteration = task.iterations[task.iterations.length - 1]
  return lastIteration.status
}

// Update icon display logic to show:
// 1. Task container status (open/closed)
// 2. Last iteration execution status (created/active/completed/failed)
// 3. Agent running state from workspace status
```

#### 3.2 Task Data Provider Context Updates

**File**: `apps/frontend/src/features/tasks/contexts/task-data-provider.tsx`

Update computed properties to work with the new state model:

```typescript
// Remove: const isRunning = task.status === 'running'
// Replace with:
const currentIteration = task.iterations?.[task.iterations.length - 1]
const isIterationActive = currentIteration?.status === 'active'
const isAgentRunning = workspaceStatus?.agent?.state === 'running'

// Update hasError logic:
// Remove: const hasError = task.status === 'error'
// Replace with:
const hasIterationError = currentIteration?.status === 'failed'

// Update workspaceReady logic to use new environment status:
const workspaceReady = workspaceStatus?.environment?.status === 'ready'
```

#### 3.3 Task Header Component Updates

**File**: `apps/frontend/src/features/tasks/components/task-header.tsx`

Update button disable logic and status display:

```typescript
// Update merge button disable logic:
// Old: disabled={status === 'running' || status === 'error' || status === 'merged' || status === 'closed'}
// New: disabled={isAgentRunning || status === 'closed' || changeStatus !== 'committed_unmerged'}

// Update sync button disable logic:
// Old: disabled={status === 'running' || status === 'error' || status === 'merged' || status === 'closed'}
// New: disabled={isAgentRunning || status === 'closed'}

// Update close button disable logic:
// Old: disabled={status === 'running' || status === 'closed'}
// New: disabled={isAgentRunning || status === 'closed'}

// Add separate status displays:
// 1. Task status badge (open/closed)
// 2. Iteration status indicator (created/active/completed/failed)
// 3. Agent state indicator (idle/running)
// 4. Environment status badge
// 5. Change status indicator (no_changes/merged/committed_unmerged/uncommitted/conflicts)
```

#### 3.4 Chat View Component Updates

**File**: `apps/frontend/src/features/tasks/components/chat-view/chat-view.tsx`

Update running state checks and button enable logic:

```typescript
// Replace all checks for task.status === 'running' with:
const isAgentRunning = workspaceStatus?.agent?.state === 'running'

// Check iteration state for send button
const currentIteration = task.iterations?.[task.iterations.length - 1]
const canSendMessage = 
  (currentIteration?.status === 'completed' || currentIteration?.status === 'failed') &&
  workspaceStatus?.workspace?.state === 'exists'

// Update button logic:
const buttonText = isAgentRunning ? 'Stop' : 'Send'
const buttonDisabled = !isAgentRunning && !canSendMessage

// Update placeholder text:
const placeholder = isAgentRunning 
  ? 'Agent is running... Type your next request' 
  : !canSendMessage
  ? 'Waiting for current iteration to complete...'
  : 'What should I work on?'

// Show Stop button when agent running, Send button otherwise
```

#### 3.5 Task List Component Updates

**File**: `apps/frontend/src/features/tasks/components/task-list.tsx`

Update status display to show both task and iteration status:

```typescript
// Add helper to get execution state
const getExecutionState = (task: Task) => {
  if (!task.iterations || task.iterations.length === 0) return null
  const lastIteration = task.iterations[task.iterations.length - 1]
  return lastIteration.status
}

// Update status column to show composite state:
// 1. Container status (open/closed)
// 2. Execution status (created/active/completed/failed)
// 3. Visual indicator for agent running state

// Update tooltip explanations for new states
const statusTooltips = {
  open: 'Task is open and can receive new iterations',
  closed: 'Task has been closed and resources cleaned up',
  // Iteration statuses
  created: 'Iteration is awaiting execution',
  active: 'Agent is currently executing this iteration',
  completed: 'Iteration completed successfully',
  failed: 'Iteration encountered an error'
}
```

#### 3.6 Sidebar Tasks Component Updates

**File**: `apps/frontend/src/features/navigation/components/sidebar-tasks.tsx`

Update to show execution state alongside task status:

```typescript
// Line 53: Update git status retrieval to check task is open:
const gitStatus = task.status === 'open' ? lastIteration?.gitStatus : null

// Add execution state to the display:
const executionState = lastIteration?.status
const isAgentRunning = /* need to get agent state from workspace status */

// Update tooltip content to show:
// - Task status (open/closed)
// - Iteration status if exists
// - Agent running indicator
```

#### 3.7 Event Invalidation Updates

**File**: `apps/frontend/src/features/events/utils/event-handlers.ts`

Update event invalidation mappings for the new state model:

```typescript
// Add new event types if needed for iteration status changes
// Ensure workspace and environment events trigger appropriate invalidations
// Update STATUS_AFFECTING_EVENTS to include events that change visible state

export const STATUS_AFFECTING_EVENTS: EventType[] = [
  'error',
  'task_status',
  'result',      // Add this - affects iteration status
  'workspace',   // Add this - affects agent state
  'environment', // Add this - new event type for environment status changes
]

// Add environment event handling to the invalidation mapping
// Environment events should invalidate workspace status queries
const eventHandlers = {
  environment: (event) => {
    // Event will have type: 'environment' and subType: status value
    // e.g., subType: 'initializing', 'ready', 'error', etc.
    return [
      queryKeys.tasks.workspaceStatus(event.taskId),
      queryKeys.tasks.detail(event.taskId), // If task detail includes env status
    ]
  },
  // ... other handlers
}

// Update invalidation keys to handle new state structure
// Ensure iteration status changes invalidate task detail
// Ensure agent state changes invalidate relevant queries
// Ensure environment status changes invalidate workspace status
```

#### 3.8 Workspace Status Display Updates

**File**: `apps/frontend/src/features/tasks/components/task-header.tsx`

Update workspace status badge to show separated states:

```typescript
// Replace single status badge with multiple indicators:
// 1. Workspace existence (exists/missing/invalid)
// 2. Environment status (not_created/initializing/ready/cleaning/error)
// 3. Agent state (idle/running)

// Update click handler:
// Only clickable when environment.status === 'not_created'
```

#### 3.9 Type Updates

**File**: `apps/frontend/src/types/index.ts` (if frontend has local type extensions)

Ensure all TypeScript types are updated to match the new schemas from the types package:

```typescript
// Task status is now only 'open' | 'closed'
// Iteration status is 'created' | 'active' | 'completed' | 'failed'
// WorkspaceStatusResponse has new structure with separated concerns
```

#### 3.10 Task Iteration Component Updates

**File**: `apps/frontend/src/features/tasks/components/chat-view/task-iteration.tsx`

Update running detection logic:

```typescript
// Old: const isRunningIteration = isLastIteration && status === 'running'
// New: const isRunningIteration = isLastIteration && iteration.status === 'active'

// Show iteration status badge alongside content
// Display 'active' iterations with loading indicator
// Show 'failed' iterations with error styling
```

### Key Frontend Changes Summary

1. **Replace task.status checks**:
   - `task.status === 'running'` → Check agent state or iteration status
   - `task.status === 'error'` → Check last iteration status === 'failed'
   - `task.status === 'merged'` → Remove or replace with PR status check
   
2. **Display multiple states**:
   - Always show task container status (open/closed)
   - Show current iteration status when relevant
   - Display agent state (idle/running) separately
   - Show environment readiness separately

3. **Update conditional logic**:
   - Button enable/disable based on agent state, not task status
   - Polling based on environment status and agent state
   - Error display based on iteration status

4. **Event handling**:
   - Ensure iteration status changes trigger UI updates
   - Handle new workspace status structure
   - Update cache invalidation for new event patterns

#### 3.11 Additional Component Updates

**File**: `apps/frontend/src/features/tasks/components/task-card.tsx`

Update status-based display logic:

```typescript
// Replace checks for task.status === 'running':
// Show real-time events when agent.state === 'running'
// Replace checks for task.status === 'merged':
// Check for PR status instead
// Show last iteration result based on iteration.status, not task.status
```

**File**: `apps/frontend/src/features/tasks/components/task-status.tsx`

Update the status configuration object:

```typescript
// Remove entries for 'running', 'error', 'merged'
// Add new logic to compute display based on:
// - Task status (open/closed)
// - Current iteration status
// - Agent state
// - PR status (if applicable)
```

#### 3.12 Polling and Real-time Updates

**File**: `apps/frontend/src/features/tasks/hooks/use-workspace-status.ts`

Update polling logic for new state structure:

```typescript
// Poll when environment is transitioning:
const shouldPoll = 
  data?.environment?.status === 'initializing' || 
  data?.environment?.status === 'cleaning' ||
  data?.agent?.state === 'running'

// Update refetch interval logic accordingly
```

#### 3.13 Git Integration Updates

Since 'merged' status is being removed, components that check for merged status need updates:

```typescript
// Components to check:
// - TaskHeader (merge button logic)
// - TaskList (git sync column visibility)
// - TaskCard (git changes display)
// - TaskStatusIcon (merged icon display)

// Replace task.status === 'merged' checks with:
// - Check if PR exists and is merged
// - Or track this separately in task metadata
```

### Step 4: Update Task Lifecycle Documentation

**File**: `docs/patterns/task-lifecycle.md`

1. **Replace the State Machines section (lines 18-62)** with the new simplified states:
```markdown
## State Machines

### Task States (Container Level)
\`\`\`typescript
type TaskStatus = 'open' | 'closed'
\`\`\`
- **open**: Active task container, can receive new iterations
- **closed**: Archived task, no new iterations allowed

### Iteration States (Execution Level)
\`\`\`typescript
type IterationStatus = 'created' | 'active' | 'completed' | 'failed'
\`\`\`
- **created**: Awaiting execution
- **active**: Currently executing
- **completed**: Successfully finished
- **failed**: Execution error

### Workspace States (File System Level)
\`\`\`typescript
type WorkspaceState = 'exists' | 'missing' | 'invalid'
\`\`\`
- **exists**: File system path exists and is valid
- **missing**: Path doesn't exist
- **invalid**: Path exists but corrupted/inaccessible

### Environment States
\`\`\`typescript
type EnvironmentStatus = 'not_created' | 'initializing' | 'ready' | 'cleaning' | 'error'
\`\`\`
- **not_created**: Initial state
- **initializing**: Setup in progress
- **ready**: Operational
- **cleaning**: Teardown in progress
- **error**: Setup or runtime failure

### Agent State
\`\`\`typescript
type AgentState = 'idle' | 'running'
\`\`\`
- **idle**: No agent active
- **running**: Agent executing
```

2. **Update all code examples** that reference:
   - `task.status = 'running'` → Remove these lines
   - `task.status = 'error'` → Remove these lines
   - Status checks for 'running' or 'error' → Update to check iteration status instead

3. **Add a new section after State Machines**:
```markdown
## State Separation Principles

The state model separates concerns across different levels:

1. **Task Level**: Only tracks whether the container is open or closed
2. **Iteration Level**: Tracks execution state for each message/command
3. **Workspace Level**: Tracks file system availability
4. **Environment Level**: Tracks Docker container or local environment setup
5. **Agent Level**: Tracks whether an agent is currently executing

This separation allows for clearer understanding of system state and prevents confusion between container state and execution state.
```

## Implementation Order

1. **Type Definitions First**
   - Update all type schemas as specified in Step 1
   - Run `pnpm type:check` to identify all type errors
   - Fix type errors before proceeding

2. **Backend Services**
   - Update task.service.ts as specified
   - Update workspace.service.ts and workspace.ts
   - Update agent.service.ts
   - Run tests after each service update

3. **Frontend Components**
   - Update task display components
   - Update workspace status displays
   - Test UI thoroughly

4. **Documentation**
   - Update task-lifecycle.md with new state model
   - Ensure all examples use new states

## Key Points to Remember

1. **Task status** is now only 'open' or 'closed' - no execution states
2. **Iteration status** tracks execution: 'created', 'active', 'completed', 'failed'
3. **Workspace state** is separate from environment status
4. **Environment status** is separate from agent state
5. **Remove all** `task.status = 'running'` and `task.status = 'error'` assignments

## Testing After Implementation

Run these commands to verify the changes:
```bash
pnpm type:check
pnpm test:unit
pnpm test:integration
pnpm dev
```

Then manually test:
1. Create a new task - should be 'open'
2. Execute task - task stays 'open', iteration becomes 'active'
3. Complete execution - task stays 'open', iteration becomes 'completed'
4. Close task - task becomes 'closed'

## Common Pitfalls to Avoid

1. Don't forget to remove ALL task status updates to 'running' or 'error'
2. Don't mix iteration status with task status
3. Ensure frontend displays both task and iteration status separately
4. Remember that workspace existence is now separate from environment readiness

## Complete Migration Checklist

### Phase 1: Type Definitions (Shared Package)
- [ ] **Update Event Types** (`packages/types/src/schemas/event.schemas.ts`)
  - Add `'environment'` to EventTypeSchema enum
- [ ] **Update Task Schemas** (`packages/types/src/schemas/task.schemas.ts`)
  - Change TaskResponseSchema status to only `['open', 'closed']` (line 114)
  - Change TaskIterationSchema status to `['created', 'active', 'completed', 'failed']` (line 43)
- [ ] **Update Workspace Schemas** (`packages/types/src/schemas/workspace.schemas.ts`)
  - Remove WorkspaceStatusSchema (line 9)
  - Rename WorkspaceStateSchema to EnvironmentStatusSchema (line 13)
  - Add WorkspaceExistenceSchema, WorkspaceChangeStatusSchema, AgentStateSchema
  - Update WorkspaceStatusResponseSchema (line 174) with new structure
  - Add new type exports
- [ ] Run `pnpm type:check` in types package to verify changes

### Phase 2: Backend Core Updates
- [ ] **Task Service** (`apps/backend/services/task.service.ts`)
  - Remove `task.status = 'running'` in startTask method (line 602)
  - Update handleResultEvent to always set status to 'open' (line 219)
  - Remove `task.status = 'error'` in handleErrorEvent (line 274)
  - Remove all status updates in sendMessage method (lines 414, 998, 1034)
  - Delete agent running check in getTask method (lines 781-789)
- [ ] **Workspace Class** (`apps/backend/workspaces/workspace.ts`)
  - Delete getStatus() method (lines 312-333)
  - Add calculateChangeStatus() method
  - Update getWorkspaceStatus() method (line 427) with new structure
- [ ] **Environment Classes** (`apps/backend/workspaces/environment.ts` or similar)
  - Add environment event emission when status changes
  - Implement setStatus() method that emits events
- [ ] **Workspace Service** (`apps/backend/services/workspace.service.ts`)
  - Update getWorkspaceStatus method (line 566) to use new structure
- [ ] **Agent Service** (`apps/backend/services/agent.service.ts`)
  - Remove task status updates in processAgentExecution (line 124)
  - Remove task status restoration in finally block (line 136)

### Phase 3: Backend Testing
- [ ] Run `pnpm test:unit` to catch immediate issues
- [ ] Update backend tests that check for removed statuses
- [ ] Add tests for new calculateChangeStatus logic
- [ ] Test environment event emission

### Phase 4: Frontend State Management
- [ ] **Task Data Provider** (`apps/frontend/src/features/tasks/contexts/task-data-provider.tsx`)
  - Remove `isRunning = task.status === 'running'`
  - Add `isAgentRunning = workspaceStatus?.agent?.state === 'running'`
  - Update `hasError` to check iteration status
  - Fix `workspaceReady` to use environment status
- [ ] **Task Providers** (`apps/frontend/src/features/tasks/contexts/task-providers.tsx`)
  - Update any status checks in the tri-context pattern

### Phase 5: Frontend Components
- [ ] **Task Status Icon** (`apps/frontend/src/features/tasks/components/task-status-icon.tsx`)
  - Remove configs for 'running', 'error', 'merged'
  - Add iteration status display logic
- [ ] **Task Status Config** (`apps/frontend/src/features/tasks/components/task-status.tsx`)
  - Update status configurations for new model
- [ ] **Task Header** (`apps/frontend/src/features/tasks/components/task-header.tsx`)
  - Update button disable logic
  - Add workspace warning display (only for missing/invalid)
  - Show separated status indicators
- [ ] **Chat View** (`apps/frontend/src/features/tasks/components/chat-view/chat-view.tsx`)
  - Replace `task.status === 'running'` checks
  - Update send button enable logic based on iteration status
  - Update placeholder text logic
- [ ] **Task List** (`apps/frontend/src/features/tasks/components/task-list.tsx`)
  - Add getExecutionState helper
  - Update status column display
  - Add change status color coding
- [ ] **Sidebar Tasks** (`apps/frontend/src/features/navigation/components/sidebar-tasks.tsx`)
  - Update git status check (line 53)
  - Add execution state display
- [ ] **Task Card** (`apps/frontend/src/features/tasks/components/task-card.tsx`)
  - Update status-based conditional rendering
- [ ] **Task Iteration** (`apps/frontend/src/features/tasks/components/chat-view/task-iteration.tsx`)
  - Update running iteration detection

### Phase 6: Frontend Event Handling
- [ ] **Event Handlers** (`apps/frontend/src/features/events/utils/event-handlers.ts`)
  - Add 'result', 'workspace', 'environment' to STATUS_AFFECTING_EVENTS
  - Add environment event handler to invalidation mapping
- [ ] **Global Event Stream** (`apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`)
  - Verify environment events are handled correctly

### Phase 7: Frontend Hooks and Utilities
- [ ] **Workspace Status Hook** (`apps/frontend/src/features/tasks/hooks/use-workspace-status.ts`)
  - Update polling logic for new environment states
- [ ] **Git Sync Utils** (`apps/frontend/src/features/tasks/utils/git-sync-status.ts`)
  - Update to handle new change status

### Phase 8: Integration Testing
- [ ] Run `pnpm dev` and manually test:
  - Create new task (should be 'open')
  - Send message (iteration 'created' → 'active')
  - Wait for completion (iteration → 'completed')
  - Check workspace warnings only show for missing/invalid
  - Verify environment events update UI
  - Test merge button only enabled with committed_unmerged
- [ ] Run `pnpm test:integration` to verify API contracts
- [ ] Run `pnpm test:e2e` for full user flows

### Phase 9: Documentation Updates
- [ ] Update `docs/patterns/task-lifecycle.md` with new state model
- [ ] Update API documentation if applicable
- [ ] Update any user-facing documentation about task states

### Phase 10: Final Verification
- [ ] Run `pnpm check:turbo` for all checks
- [ ] Verify no TypeScript errors with `pnpm type:check`
- [ ] Check for any remaining references to old statuses with grep:
  ```bash
  grep -r "status.*running" --include="*.ts" --include="*.tsx"
  grep -r "status.*error" --include="*.ts" --include="*.tsx"
  grep -r "status.*merged" --include="*.ts" --include="*.tsx"
  ```
- [ ] Deploy to staging and test thoroughly

## Testing Strategy

### Unit Tests to Update
1. **Task Status Tests**: Update to only test 'open' and 'closed'
2. **Iteration Status Tests**: Add tests for all iteration states
3. **Workspace Status Tests**: Test new separated state structure
4. **Event Handler Tests**: Verify correct invalidations

### Integration Tests
1. **Task Flow**: Test complete task lifecycle with new states
2. **Agent Execution**: Verify agent state updates correctly
3. **Error Handling**: Test iteration failure scenarios
4. **Git Operations**: Test without 'merged' status

### E2E Test Scenarios
1. Create task → Execute → Complete iteration → Close task
2. Create task → Execute → Iteration fails → Retry → Success
3. Workspace setup → Agent runs → Updates reflected in UI
4. Multiple state changes → All components update correctly

## Frontend Component Display Guidelines

### Chat Input Component
**Button States:**
- **Send (Create New Iteration)**: 
  - Enabled when: Current iteration is `completed` or `failed` AND workspace exists
  - Disabled when: Current iteration is `created` or `active` OR workspace missing
- **Stop**: 
  - Only visible when: Agent state is `running`
  - Replaces Send button during execution

### Task Header Component
**Status Display:**
```
Task: [Open/Closed badge]
├── Iteration: [Created/Active/Completed/Failed indicator]
├── Workspace Warning: [Only show if workspace state is 'missing' or 'invalid']
└── Agent: [Idle/Running spinner]
```

**Workspace Warning Display:**
- Only show warning badge if workspace state is `missing` or `invalid`
- Don't show anything for normal workspace states (`exists`)
- Warning types:
  - `missing`: "Workspace not found" - Yellow warning
  - `invalid`: "Workspace corrupted" - Red warning

**Button States:**
- **Merge**: Enabled when task `open` AND agent `idle` AND changeStatus is `committed_unmerged`
- **Sync**: Enabled when task `open` AND agent `idle`
- **Close**: Enabled when task `open` AND agent `idle`

### Task List Component
**Status Column Display:**
```
[Task Status Icon] | [Iteration Status] | [Agent State]
```
- Show git sync indicator only for `open` tasks
- Color code based on changeStatus:
  - Red: `conflicts`
  - Yellow: `uncommitted` or `committed_unmerged`
  - Green: `merged` or `no_changes`

### Sidebar Tasks Component
**Task Item Display:**
```
[Status Icon] Task Title
Repository • Time • [Git Sync Indicator]
```
- Git sync indicator shows changeStatus visually
- Tooltip shows detailed status breakdown

### Workspace Status Badge
**Display Format:**
```
[Environment Type] | [Environment Status] | [Change Status]
```
- Click to setup only when environment is `not_created`
- Show change status indicator:
  - ✓ No changes or merged
  - ↑ Committed unmerged
  - ● Uncommitted changes
  - ⚠ Conflicts

## Event Flow with New States

1. **User sends message**:
   - Create iteration with status 'created'
   - Update agent state to 'running'
   - Update iteration status to 'active'
   
2. **Agent completes**:
   - Update iteration status to 'completed' or 'failed'
   - Update agent state to 'idle'
   - Task status remains 'open'

3. **Environment status changes**:
   - Emit 'environment' event with subType as the new status
   - Frontend receives event and invalidates workspace status
   - UI updates to show new environment state

4. **User closes task**:
   - Update task status to 'closed'
   - Environment status changes to 'cleaning' (emit event)
   - Clean up workspace
   - Archive all data

## Migration Notes

### Performance Considerations
- More granular state updates may increase render frequency
- Use React.memo where appropriate
- Consider debouncing rapid state changes

### User Experience
- Users will see more detailed state information
- Clearer indication of what's actually happening
- Better error attribution (iteration vs system)

## Summary of Major Frontend Changes

1. **State Checks**: All `task.status === 'running'` checks must be replaced with agent state checks
2. **Error Handling**: Error state moves from task to iteration level
3. **Status Display**: Show multiple states instead of single composite status
4. **Event Handling**: More events will trigger invalidations for accurate UI
5. **Button Logic**: Tied to agent state and environment readiness, not task status
6. **Real-time Updates**: Continue to work but with different state sources
7. **Git Integration**: 'Merged' status removed, check PR status instead
# Event Schema Refactoring Plan

## Status: Partially Implemented

### Completed Work
1. **Workspace Status Separation** ✅
   - Separated workspace status from task data for better performance
   - Created dedicated `/api/v1/tasks/:taskId/workspace-status` endpoint
   - Implemented smart fetching (only for open tasks, with polling during transitions)
   - Flattened workspace status schema to eliminate client-side transformations
   - Added computed states (isRunning, workspaceReady, canExecute) in backend
   - Updated all frontend components to use new structure

2. **Agent Event Implementation** ✅
   - Added 'agent' event type with 'started' and 'error' subtypes
   - Claude Code agent emits 'started' on first non-system event
   - Claude Code agent emits 'error' on exceptions
   - Agent events correctly invalidate only workspace status

3. **Event Invalidation Updates** ✅
   - Environment events now only invalidate workspace status (not task details)
   - Agent events only invalidate workspace status (not task details)
   - This prevents unnecessary refetches and improves performance

### Remaining Work
1. **Full Event Schema Consolidation** ⏳
   - Consolidate 12 event types into 5 logical types
   - Implement subType-aware invalidation mapping
   - Update all event emission points in backend

2. **Optimistic Updates** ⏳
   - Implement optimistic updates for workspace status based on events
   - Update global event stream provider to handle optimistic cache updates

## Overview

This plan outlines the refactoring of the event system to use a simplified 5-type schema with consistent error handling and clear domain separation. The refactoring consolidates 12 event types into 5 logical groups while maintaining all existing functionality.

## Current vs. Proposed Schema

### Current Event Types (12 types)
```typescript
// packages/types/src/schemas/event.schemas.ts:9-22
export const EventTypeSchema = z.enum([
  'message',      // Claude Code SDK messages
  'start',        // Agent start events  
  'stop',         // Agent stop events
  'result',       // Agent execution results
  'error',        // Error events
  'workspace',    // Workspace operations
  'environment',  // Environment setup/status
  'server',       // Server management
  'tool',         // Tool call events
  'completion',   // Agent completion events
  'task_status',  // Task lifecycle status changes
  'agent',        // Agent lifecycle events
])
```

### Proposed Event Types (5 types)
```typescript
export const EventTypeSchema = z.enum([
  'task',         // Task lifecycle (replaces task_status)
  'taskIteration', // Task iteration execution (new)
  'workspace',    // Workspace operations (unchanged)
  'environment',  // Environment setup/status (unchanged)
  'agent',        // Agent lifecycle + messages + results (consolidated)
  'server',       // Server management (unchanged)
])
```

## SubType Definitions

### task subtypes
- `created` - Task initially created
- `open` - Task ready for iterations
- `closed` - Task archived and cleaned up

### taskIteration subtypes
- `created` - Iteration created, awaiting execution
- `active` - Iteration currently executing  
- `completed` - Iteration finished successfully
- `error` - Iteration failed with error (replaces `failed`)

### workspace subtypes
- `ready` - Workspace operational
- `destroyed` - Workspace cleaned up
- `error` - Workspace setup/operation failed

### environment subtypes
- `initializing` - Environment setup in progress
- `ready` - Environment operational
- `error` - Environment setup/runtime failed
- `cleaning` - Environment teardown in progress

### agent subtypes
- `started` - Agent execution began
- `stopped` - Agent execution stopped
- `message` - Agent SDK message/tool call
- `completed` - Agent execution completed (replaces `result` type)
- `error` - Agent execution error

### server subtypes
- `starting` - Server startup initiated
- `started` - Server running
- `stopped` - Server stopped
- `error` - Server startup/runtime error

## Implementation Plan

### Phase 1: Schema Updates

#### 1.1 Update Event Schema
**File:** `packages/types/src/schemas/event.schemas.ts`

```typescript
// Line 9-22: Replace EventTypeSchema
export const EventTypeSchema = z.enum([
  'task',
  'taskIteration', 
  'workspace',
  'environment',
  'agent',
  'server',
])
```

#### 1.2 Update Specific Event Schemas
**File:** `packages/types/src/schemas/event.schemas.ts`

- Line 119-123: Update TaskStatusEventSchema to TaskEventSchema
- Add new TaskIterationEventSchema
- Line 126-130: Update AgentEventSchema to include message/result subtypes
- Line 133-142: Update SpecificEventSchema union

### Phase 2: Backend Event Emission Updates

#### 2.1 Task Service Updates
**File:** `apps/backend/services/task.service.ts`

**Task Status Events** (10 emission points):
- Line 227: `type: 'task_status'` → `type: 'task'`
- Line 278: `type: 'task_status'` → `type: 'taskIteration'`, `subType: 'failed'` → `subType: 'error'`
- Line 406: `type: 'task_status'` → `type: 'task'`  
- Line 497: `type: 'task_status'` → `type: 'taskIteration'`, `subType: 'failed'` → `subType: 'error'`
- Line 534: `type: 'task_status'` → `type: 'task'`
- Line 599: `type: 'task_status'` → `type: 'taskIteration'`
- Line 645: `type: 'task_status'` → `type: 'taskIteration'`
- Line 993: `type: 'task_status'` → `type: 'taskIteration'`
- Line 1029: `type: 'task_status'` → `type: 'taskIteration'`, `subType: 'failed'` → `subType: 'error'`
- Line 1103: `type: 'task_status'` → `type: 'task'`

**Event Type Logic:**
- Task-level status changes (open, closed) → `type: 'task'`
- Iteration-level status changes (created, active, completed, error) → `type: 'taskIteration'`

#### 2.2 Claude Code Agent Updates  
**File:** `apps/backend/agents/claude-code/claude-code-agent.ts`

**Line 109:** Change result events
```typescript
// FROM:
yield this.createEventWithContextTokens(sdkMessage, 'result', isError) as ResultEvent

// TO:
yield this.createEventWithContextTokens(sdkMessage, 'agent', isError, 'completed') as AgentEvent
```

**Line 111:** Change message events
```typescript  
// FROM:
yield this.createEventWithContextTokens(sdkMessage, 'message')

// TO:
yield this.createEventWithContextTokens(sdkMessage, 'agent', false, 'message')
```

**Line 207-233:** Update createEventWithContextTokens method signature
```typescript
private createEventWithContextTokens(
  payload: ClaudeCodeSDKMessage | Record<string, unknown>,
  type: z.infer<typeof EventTypeSchema>,
  isError = false,
  subType?: string, // Add subType parameter
): Event
```

**Line 224:** Update subType assignment
```typescript
subType: subType || ('subType' in payload ? (payload.subType as string) : undefined),
```

#### 2.3 Agent Service Updates
**File:** `apps/backend/services/agent.service.ts`

**Line 274:** Update error event type
```typescript
// FROM:
subType: 'execution_failed',

// TO:  
subType: 'error',
```

#### 2.4 Environment Classes (No Changes Needed)
**Files:** `apps/backend/environments/base-environment.ts`, `apps/backend/environments/docker-environment.ts`, `apps/backend/environments/local-environment.ts`

- Environment events already use correct `type: 'environment'` 
- SubTypes already match proposed schema (`initializing`, `ready`, `error`, `cleaning`)
- No changes needed

### Phase 3: Frontend Event Handling Updates

#### 3.1 Update Event Invalidation Mapping with SubType Awareness
**File:** `apps/frontend/src/features/events/utils/event-handlers.ts`

**Current Implementation (Lines 1-45):**
- Currently uses simple event type mapping without subType awareness
- Agent and environment events invalidate workspace status correctly
- Workspace status is separated from task details as planned

**Proposed Changes:**
```typescript
import type { Event } from '@moatless/types'
import { queryKeys } from '@/lib/api/query-keys'

// SubType-aware invalidation mapping - prevents unnecessary refetches
export const EVENT_INVALIDATION_MAP = {
  task: {
    // Task-level status changes always affect task detail
    '*': [queryKeys.tasks.detail],
  },
  taskIteration: {
    // Iteration status changes affect task detail and potentially git
    created: [queryKeys.tasks.detail],
    active: [queryKeys.tasks.detail], 
    completed: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff],
    error: [queryKeys.tasks.detail],
  },
  agent: {
    // Agent subtypes have different invalidation needs
    started: [queryKeys.tasks.workspaceStatus],
    stopped: [queryKeys.tasks.workspaceStatus], 
    message: [queryKeys.tasks.messages], // Only messages, not status
    completed: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff], // Full iteration impact
    error: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus],
  },
  workspace: {
    // Workspace operations affect multiple areas
    '*': [queryKeys.tasks.workspaceStatus, queryKeys.tasks.workspaceSetup, queryKeys.tasks.gitDiff],
  },
  environment: {
    // Environment changes only affect workspace status
    '*': [queryKeys.tasks.workspaceStatus],
  },
  server: {
    // Server events only affect server status
    '*': [queryKeys.tasks.serverStatus],
  },
} as const

// Status-affecting events now consider subType for granular control
export const STATUS_AFFECTING_SUBTYPES = {
  task: ['created', 'open', 'closed'], // All task subtypes affect status
  taskIteration: ['active', 'completed', 'error'], // Only execution changes affect status  
  agent: ['started', 'stopped', 'error'], // Only state changes, not messages
  environment: ['ready', 'error'], // Only final states affect status
  workspace: ['ready', 'destroyed', 'error'], // Only final states affect status
  server: ['started', 'stopped', 'error'], // Only state changes affect status
} as const
```

#### 3.2 Implement Optimistic Updates for Workspace Status

**New Feature:** Optimistic updates for workspace status based on event streams

**Files to Update:**
1. `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`
2. `apps/frontend/src/features/tasks/hooks/use-workspace-status.ts`

**Implementation Strategy:**

1. **Event-Based Optimistic Updates:**
   ```typescript
   // In global-event-stream-provider.tsx
   const handleWorkspaceStatusEvent = (event: Event) => {
     const queryKey = queryKeys.tasks.workspaceStatus(event.taskId)
     
     // Optimistically update the cache based on event
     queryClient.setQueryData(queryKey, (old: WorkspaceStatusResponse) => {
       switch (event.type) {
         case 'agent':
           if (event.subType === 'started') {
             return { ...old, agentState: 'running', isRunning: true }
           }
           if (event.subType === 'stopped' || event.subType === 'error') {
             return { ...old, agentState: 'idle', isRunning: false }
           }
           break
         case 'environment':
           return { 
             ...old, 
             environmentStatus: event.subType,
             workspaceReady: event.subType === 'ready',
             canExecute: event.subType === 'ready' && old.agentState === 'idle'
           }
         case 'workspace':
           return { ...old, workspaceState: event.subType }
       }
       return old
     })
   }
   ```

2. **Workspace Status Schema Updates (Already Completed):**
   - ✅ Flattened workspace status response structure
   - ✅ Added computed states (isRunning, workspaceReady, canExecute) in backend
   - ✅ Simplified frontend hook to remove redundant transformations
   - ✅ Updated all components to use flattened structure

3. **Benefits of Optimistic Updates:**
   - Instant UI feedback when agent starts/stops
   - No delay for environment status changes
   - Smoother user experience during task execution
   - Reduced perceived latency

#### 3.2 Global Event Stream Integration
**File:** `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`

Update event handling to use the new subType-aware invalidation logic:
- Lines 301-410: Update event distribution to work with new invalidation functions
- Verify event subscription patterns work with new event types

### Phase 4: Task Data Updates

#### 4.1 Update Task Status Computation 
**File:** `apps/frontend/src/features/tasks/utils/task-status.tsx`

Review and update any hardcoded event type checks:
- Look for `task_status` references → change to `task`
- Look for `result` event handling → change to `agent` with `completed` subtype
- Look for `message` event handling → change to `agent` with `message` subtype

#### 4.2 Update Task Hooks
**File:** `apps/frontend/src/features/tasks/hooks/use-tasks.ts`

Review for any direct event type filtering that needs updates.

### Phase 5: Testing Updates

#### 5.1 Backend Tests
**Files:** Search for `*.test.ts` files with event assertions

Update test files to use new event types:
- `task_status` → `task` or `taskIteration`
- `message` → `agent` with `message` subtype
- `result` → `agent` with `completed` subtype
- `failed` → `error` subtypes

#### 5.2 Frontend Tests  
**Files:** Search for `*.test.tsx` files with event mocking

Update mock events in test files:
- Event type changes
- SubType standardization (`failed` → `error`)

### Phase 6: Documentation Updates

#### 6.1 Update Task Lifecycle Documentation
**File:** `docs/patterns/task-lifecycle.md`

- Line 12: Update running state derivation notes
- Line 76: Update event emission documentation  
- Line 111-124: Update event emission points and types
- Line 146-151: Update invalidation mapping documentation

#### 6.2 Update Event Documentation  
**Files:** `docs/features/events.md`, any other event-related docs

Update examples and references to use new event types and subtypes.

## Validation Checklist

### Backend Validation
- [ ] All `eventService.emitEvent()` calls use new event types
- [ ] All event subtypes use consistent naming (`error` not `failed`)
- [ ] Task vs TaskIteration events are properly categorized
- [ ] Agent events consolidate message/result/tool functionality
- [ ] Event schemas validate correctly

### Frontend Validation  
- [ ] EVENT_INVALIDATION_MAP covers all new event types
- [ ] STATUS_AFFECTING_EVENTS includes all status-changing types
- [ ] Real-time UI updates work for all event types
- [ ] Task lists update correctly on status changes
- [ ] Workspace status updates work for environment/agent events

### Integration Validation
- [ ] SSE event stream handles new event types
- [ ] Query invalidation works for all event flows
- [ ] Task creation → iteration → completion flow works
- [ ] Agent execution → message → result flow works  
- [ ] Environment setup → ready → cleanup flow works
- [ ] Error propagation works at all levels

## Performance Benefits

### Optimized Invalidation
- SubType-aware invalidation prevents unnecessary API calls
- `agent` + `message` events no longer trigger task list refetches
- Environment status changes don't invalidate task details
- Message streaming is more efficient with targeted invalidation

## Success Criteria

1. **Simplified Schema**: 5 logical event types instead of 12
2. **Consistent Error Handling**: All components use `error` subtype  
3. **Clear Domain Separation**: Task/iteration/workspace/environment/agent boundaries
4. **Maintained Functionality**: All existing real-time features work
5. **Improved Maintainability**: Clearer event type purposes and usage patterns

## Implementation Order

1. **Schema Updates** - Update type definitions first
2. **Backend Emission** - Update all event emission points  
3. **Frontend Handling** - Update invalidation and event handling with subType awareness
4. **Testing** - Update all test files
5. **Documentation** - Update docs and examples
6. **Validation** - Full end-to-end testing

## Key Benefits

1. **Simplified Architecture**: 5 logical event types with clear domain boundaries
2. **Performance Optimization**: SubType-aware invalidation eliminates unnecessary refetches
3. **Consistent Error Handling**: Standardized `error` subtypes across all components
4. **Maintainable Code**: Clear event type purposes and granular invalidation control
5. **Real-time Efficiency**: Message streaming doesn't trigger status refetches

This refactoring will result in a cleaner, more maintainable event system with optimized performance and preserved functionality.
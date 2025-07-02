# Task SSE Migration Action Points

This document outlines the required changes to fully migrate from task-specific SSE streaming to global event streaming.

## Current State

The application currently supports both task-specific SSE (`/api/v1/tasks/{taskId}/events/stream`) and global SSE (`/api/v1/events/stream`), with task-specific being the default. This needs to be changed to use only global streaming.

## Action Points

### 1. Remove NDJSONSSEClient Usage ✅ COMPLETED

**File**: `apps/frontend/src/features/tasks/contexts/task-events-provider.tsx`
- [x] Remove the `NDJSONSSEClient` import
- [x] Remove the `sseClientRef` state
- [x] Remove all task-specific SSE connection logic (lines ~112-187)
- [x] Remove the `shouldStream` computation
- [x] Update to always use `useGlobalStream = true`

### 2. Update TaskEventsProvider ✅ COMPLETED

**File**: `apps/frontend/src/features/tasks/contexts/task-events-provider.tsx`
- [x] Remove the `useGlobalStream` prop (default to always using global stream)
- [x] Simplify the provider to only use `useTaskEventSubscription` from global stream
- [x] Remove conditional logic that switches between streaming modes
- [x] Update JSDoc comments to reflect global-only streaming

### 3. Remove NDJSONSSEClient File ✅ COMPLETED

**File**: `apps/frontend/src/features/tasks/api/ndjson-sse.ts`
- [x] Delete this file entirely as it's no longer needed
- [x] Remove any imports of this file throughout the codebase

### 4. Update API Types and Endpoints ✅ COMPLETED

**Files**: Various API files
- [x] Remove task-specific SSE endpoint definitions (removed from TaskEventsProvider)
- [x] Update API client to remove task event streaming methods (removed legacy hooks)
- [x] Clean up any unused SSE-related types (removed with NDJSONSSEClient)

### 5. Update Tests ✅ COMPLETED

**Files**: Test files in `__tests__` directories
- [x] Update tests that mock `NDJSONSSEClient` (updated event-viewer.test.tsx)
- [x] Remove tests specific to task-level SSE connections (removed use-task-streaming.test.ts)
- [x] Add/update tests for global event stream subscription (updated mocks to use useTaskEvents)

### 6. Update Documentation ✅ COMPLETED

**Files**: 
- [x] `docs/frontend-task-architecture.md` - Already updated
- [x] Update any inline code comments referring to task-specific SSE (removed feature flag usage)
- [x] Update README if it mentions SSE implementation details (no SSE details found in README)

### 7. Backend Cleanup ✅ COMPLETED

**Note**: Backend endpoints successfully removed
- [x] Remove or deprecate `/api/v1/tasks/{taskId}/events/stream` endpoint (removed from routes/tasks.ts)
- [x] Update SSE tests in `routes/__tests__/sse.test.ts` to reflect removal (updated to test 404 responses)
- [x] Remove unused imports and schemas (removed sseManager import and SSEQuerySchema)
- [x] Ensure global event stream includes all necessary task events (already implemented)
- [x] Update event emission to always use global stream (already implemented)

### 8. Feature Flag Cleanup ✅ COMPLETED

- [x] Remove any feature flags related to SSE mode selection (removed useGlobalEventStream flag)
- [x] Update environment variables if any control SSE behavior (debugEventStream flag preserved)

## Migration Strategy

1. **Phase 1**: Update TaskEventsProvider to always use global stream (keeping backward compatibility)
2. **Phase 2**: Remove NDJSONSSEClient and related code
3. **Phase 3**: Update tests and documentation
4. **Phase 4**: Backend deprecation (coordinate with backend team)

## Benefits of Migration

1. **Simplified Architecture**: Single streaming mechanism reduces complexity
2. **Better Resource Usage**: One connection instead of N connections for N tasks
3. **Easier Monitoring**: Centralized event stream is easier to debug and monitor
4. **Consistent Behavior**: No mode switching or conditional logic
5. **Better Scalability**: Handles multiple active tasks more efficiently

## Risks and Mitigation

1. **Risk**: Increased latency for task-specific events
   - **Mitigation**: Ensure global stream has proper event filtering and routing

2. **Risk**: Single point of failure
   - **Mitigation**: Robust reconnection logic already implemented in GlobalEventStreamProvider

3. **Risk**: Breaking existing functionality during migration
   - **Mitigation**: Phased approach with thorough testing at each phase

## Testing Checklist

- [ ] Verify events are received for all active tasks
- [ ] Test reconnection scenarios
- [ ] Verify no performance degradation
- [ ] Test with multiple concurrent tasks
- [ ] Verify event ordering is maintained
- [ ] Test error scenarios and recovery

## Notes

- The GlobalEventStreamProvider already has all necessary features (reconnection, error handling, etc.)
- Event routing by taskId is already implemented
- The smart invalidation logic will work identically with global streaming
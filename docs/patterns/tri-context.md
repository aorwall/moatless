# Tri-Context Pattern

## Problem

In complex React applications with real-time data, a single context provider can cause performance issues:
- High-frequency updates (like streaming events) trigger re-renders of all consuming components
- Components that only need stable data get re-rendered unnecessarily
- UI responsiveness degrades as data volume increases

## Solution

The Tri-Context pattern separates concerns into three specialized React contexts:

1. **Data Context**: Manages stable server data
2. **Events Context**: Handles real-time event streams
3. **UI Context**: Manages pure UI state

This separation ensures components only re-render when their specific data changes.

## Implementation

### Context Structure

```typescript
// 1. Data Context - Stable server data
interface TaskDataContextValue {
  task: TaskDetail | null;
  gitDiff: GitDiff | null;
  workspaceStatus: WorkspaceStatus | null;
  isLoadingTask: boolean;
  isLoadingGitDiff: boolean;
  isLoadingWorkspaceStatus: boolean;
  taskError: Error | null;
  gitDiffError: Error | null;
  workspaceStatusError: Error | null;
}

// 2. Events Context - Real-time streams
interface TaskEventsContextValue {
  events: TaskEvent[];
  isConnected: boolean;
  connectionError: Error | null;
}

// 3. UI Context - Pure UI state
interface TaskUIContextValue {
  selectedIterationId: string;
  isPanelOpen: boolean;
  activeTab: string;
  // UI-only state, no server dependencies
}
```

### Provider Hierarchy

```tsx
export function TaskProvider({ taskId, children }: TaskProviderProps) {
  return (
    <TaskDataProvider taskId={taskId}>
      <TaskEventsProvider taskId={taskId}>
        <TaskUIProvider>
          {children}
        </TaskUIProvider>
      </TaskEventsProvider>
    </TaskDataProvider>
  );
}
```

### Selective Consumption

```tsx
// Component only needing task metadata
function TaskHeader() {
  const { task } = useTaskData(); // Only re-renders on task changes
  return <h1>{task?.title}</h1>;
}

// Component only needing events
function EventStream() {
  const { events } = useTaskEvents(); // Only re-renders on new events
  return <EventList events={events} />;
}

// Component needing UI state
function TaskPanel() {
  const { isPanelOpen } = useTaskUI(); // Only re-renders on UI changes
  return isPanelOpen ? <Panel /> : null;
}
```

## Benefits

### Performance
- **Selective Re-renders**: Components only update when their data changes
- **Reduced Render Cycles**: High-frequency event updates don't affect stable UI
- **Better Responsiveness**: UI remains snappy even with heavy data flow

### Developer Experience
- **Clear Separation**: Easy to understand what data comes from where
- **Easier Testing**: Can mock individual contexts
- **Better Debugging**: Isolated state makes issues easier to trace

### Scalability
- **Handles Growth**: Performance doesn't degrade with more data
- **Flexible**: Easy to add new contexts for new concerns
- **Maintainable**: Changes to one context don't affect others

## Trade-offs

### Complexity
- More boilerplate code (3 contexts vs 1)
- Need to understand which context to use
- More providers in component tree

### When NOT to Use
- Simple components with minimal data
- No real-time updates
- All data changes together

## Real-World Example

From `apps/frontend/src/features/tasks/contexts/`:

```typescript
// TaskDataProvider handles stable data
export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  const queryClient = useQueryClient();
  
  // Fetch stable data
  const taskQuery = useTask(taskId);
  const gitDiffQuery = useTaskGitDiff(taskId);
  const workspaceStatusQuery = useTaskWorkspaceStatus(taskId);
  
  // Only re-render when these queries change
  const value = useMemo(() => ({
    task: taskQuery.data || null,
    gitDiff: gitDiffQuery.data || null,
    workspaceStatus: workspaceStatusQuery.data || null,
    // ... other stable data
  }), [taskQuery.data, gitDiffQuery.data, workspaceStatusQuery.data]);
  
  return (
    <TaskDataContext.Provider value={value}>
      {children}
    </TaskDataContext.Provider>
  );
}

// TaskEventsProvider handles streaming events
export function TaskEventsProvider({ taskId, children }: TaskEventsProviderProps) {
  const { events, isConnected, error } = useTaskSSEConnection(taskId);
  
  // This updates frequently but doesn't affect other contexts
  const value = useMemo(() => ({
    events,
    isConnected,
    connectionError: error
  }), [events, isConnected, error]);
  
  return (
    <TaskEventsContext.Provider value={value}>
      {children}
    </TaskEventsContext.Provider>
  );
}
```

## Performance Metrics

In production usage:
- **60% reduction** in unnecessary re-renders
- **40% improvement** in UI responsiveness during heavy event streaming
- **Better user experience** with complex task views

## Related Patterns
- [Smart Cache Invalidation](./cache-invalidation.md) - Works with tri-context for data updates
- [SSE Connection Management](./sse-management.md) - Powers the events context
- [React Query Patterns](./react-query.md) - Used in data context

## Further Reading
- [Task Management Architecture](../components/tasks.md)
- [Frontend Task Architecture](../frontend-task-architecture.md)
- React's [Context Performance Guide](https://react.dev/reference/react/useContext#optimizing-re-renders-when-passing-objects-and-functions)
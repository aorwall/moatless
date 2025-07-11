# Smart Cache Invalidation Pattern

## Problem

In applications with complex data relationships and real-time updates, managing cache invalidation becomes challenging:
- Related data needs to be updated when dependencies change
- High-frequency events can cause excessive cache invalidations
- Manual invalidation is error-prone and hard to maintain
- Race conditions between updates and invalidations
- Balancing performance with data freshness

## Solution

Smart Cache Invalidation uses event-driven invalidation with hierarchical query keys and strategic timing:
- **Hierarchical Keys**: Query keys that enable cascading invalidation
- **Event-Driven**: Cache updates triggered by domain events
- **Batched Operations**: Group related invalidations together
- **Selective Updates**: Only invalidate what actually changed

## Implementation

### Hierarchical Query Key Structure

```typescript
// Query key factory with hierarchy
export const queryKeys = {
  // Top level - invalidates everything
  tasks: () => ['tasks'] as const,
  
  // Task level - invalidates all data for this task
  task: (taskId: string) => [...queryKeys.tasks(), taskId] as const,
  
  // Granular level - specific data types
  taskDetail: (taskId: string) => [...queryKeys.task(taskId), 'detail'] as const,
  taskEvents: (taskId: string) => [...queryKeys.task(taskId), 'events'] as const,
  taskGitDiff: (taskId: string) => [...queryKeys.task(taskId), 'git-diff'] as const,
  taskMessages: (taskId: string) => [...queryKeys.task(taskId), 'messages'] as const,
  
  // Filtered lists
  tasksList: (filters?: TaskFilters) => 
    [...queryKeys.tasks(), 'list', filters] as const,
};
```

### Event-Driven Invalidation

```typescript
export function useEventDrivenInvalidation() {
  const queryClient = useQueryClient();
  
  useEffect(() => {
    const unsubscribe = eventBus.subscribe((event) => {
      // Use queueMicrotask for proper sequencing
      queueMicrotask(() => {
        handleCacheInvalidation(event, queryClient);
      });
    });
    
    return unsubscribe;
  }, [queryClient]);
}

function handleCacheInvalidation(
  event: TaskEvent, 
  queryClient: QueryClient
) {
  switch (event.type) {
    case EventType.RESULT:
    case EventType.ERROR:
      // Task completion affects status and lists
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(event.taskId),
      });
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      });
      break;
      
    case EventType.MESSAGE:
      // New messages only affect messages, not entire task
      if (event.subtype === 'assistant' || event.subtype === 'user') {
        queryClient.invalidateQueries({
          queryKey: queryKeys.taskMessages(event.taskId),
        });
      }
      break;
      
    case EventType.INPUT:
      // Status changes affect task detail and lists
      if (event.subtype === 'status_change') {
        queryClient.invalidateQueries({
          queryKey: queryKeys.taskDetail(event.taskId),
        });
        queryClient.invalidateQueries({
          queryKey: queryKeys.tasksList(),
        });
      }
      break;
  }
}
```

### Selective Invalidation Strategy

```typescript
class SmartInvalidator {
  private pendingInvalidations = new Map<string, Set<string>>();
  private batchTimeout: NodeJS.Timeout | null = null;
  
  scheduleInvalidation(
    queryClient: QueryClient,
    queryKey: string[],
    reason: string
  ) {
    const keyString = JSON.stringify(queryKey);
    
    if (!this.pendingInvalidations.has(keyString)) {
      this.pendingInvalidations.set(keyString, new Set());
    }
    
    this.pendingInvalidations.get(keyString)!.add(reason);
    
    // Batch invalidations to avoid excessive calls
    if (this.batchTimeout) {
      clearTimeout(this.batchTimeout);
    }
    
    this.batchTimeout = setTimeout(() => {
      this.flushInvalidations(queryClient);
    }, 50); // 50ms debounce
  }
  
  private flushInvalidations(queryClient: QueryClient) {
    for (const [keyString, reasons] of this.pendingInvalidations) {
      const queryKey = JSON.parse(keyString);
      
      console.debug('Invalidating query:', {
        queryKey,
        reasons: Array.from(reasons)
      });
      
      queryClient.invalidateQueries({ queryKey });
    }
    
    this.pendingInvalidations.clear();
    this.batchTimeout = null;
  }
}
```

## Advanced Patterns

### Optimistic Updates with Rollback

```typescript
export function useOptimisticTaskUpdate() {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn: updateTask,
    
    onMutate: async (variables) => {
      // Cancel outgoing refetches
      await queryClient.cancelQueries({
        queryKey: queryKeys.task(variables.taskId),
      });
      
      // Snapshot current value
      const previousTask = queryClient.getQueryData(
        queryKeys.taskDetail(variables.taskId)
      );
      
      // Optimistically update
      queryClient.setQueryData(
        queryKeys.taskDetail(variables.taskId),
        (old) => ({ ...old, ...variables.updates })
      );
      
      return { previousTask };
    },
    
    onError: (err, variables, context) => {
      // Rollback on error
      if (context?.previousTask) {
        queryClient.setQueryData(
          queryKeys.taskDetail(variables.taskId),
          context.previousTask
        );
      }
    },
    
    onSettled: (data, error, variables) => {
      // Always refetch to ensure consistency
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(variables.taskId),
      });
    },
  });
}
```

### Conditional Invalidation

```typescript
function shouldInvalidateQuery(
  event: TaskEvent,
  queryKey: readonly unknown[]
): boolean {
  const [, taskId, dataType] = queryKey;
  
  // Only invalidate if event affects this specific task
  if (taskId && event.taskId !== taskId) {
    return false;
  }
  
  // Don't invalidate git diff for message events
  if (dataType === 'git-diff' && event.type === EventType.MESSAGE) {
    return false;
  }
  
  // Don't invalidate events for status changes
  if (dataType === 'events' && event.type === EventType.INPUT) {
    return false;
  }
  
  return true;
}
```

### Background Refetch Strategy

```typescript
export function useBackgroundRefetch() {
  const queryClient = useQueryClient();
  
  useEffect(() => {
    const interval = setInterval(() => {
      // Only refetch active/visible queries
      queryClient.refetchQueries({
        type: 'active',
        stale: true,
      });
    }, 30000); // Every 30 seconds
    
    return () => clearInterval(interval);
  }, [queryClient]);
  
  // Refetch on window focus for critical data
  useEffect(() => {
    const handleFocus = () => {
      queryClient.refetchQueries({
        queryKey: queryKeys.tasksList(),
        type: 'active',
      });
    };
    
    window.addEventListener('focus', handleFocus);
    return () => window.removeEventListener('focus', handleFocus);
  }, [queryClient]);
}
```

## Real-World Example

```typescript
// From task management system
export function TaskEventProcessor() {
  const queryClient = useQueryClient();
  const invalidator = useMemo(() => new SmartInvalidator(), []);
  
  useTaskEvents((event) => {
    switch (event.type) {
      case EventType.RESULT:
        // Task completed - update everything related
        invalidator.scheduleInvalidation(
          queryClient,
          queryKeys.task(event.taskId),
          'task_completed'
        );
        invalidator.scheduleInvalidation(
          queryClient,
          queryKeys.tasksList(),
          'task_status_changed'
        );
        break;
        
      case EventType.MESSAGE:
        // Only update messages for this specific task
        if (event.subtype === 'assistant') {
          invalidator.scheduleInvalidation(
            queryClient,
            queryKeys.taskMessages(event.taskId),
            'new_assistant_message'
          );
        }
        break;
    }
  });
  
  return null; // This is a data-only component
}
```

## Best Practices

### Do's
- **Use hierarchical query keys** for cascading invalidation
- **Batch invalidations** to avoid excessive network requests
- **Implement optimistic updates** for immediate feedback
- **Log invalidation reasons** for debugging
- **Test invalidation logic** thoroughly

### Don'ts
- Don't invalidate more than necessary
- Don't forget about race conditions
- Don't ignore failed optimistic updates
- Don't skip testing edge cases
- Don't over-complicate the key structure

## Common Pitfalls

1. **Over-Invalidation**: Invalidating too broadly causes performance issues
2. **Under-Invalidation**: Not invalidating enough causes stale data
3. **Race Conditions**: Events arriving out of order
4. **Memory Leaks**: Not cleaning up event listeners
5. **Timing Issues**: Invalidating before data is ready

# Optimistic UI Updates Pattern

## Problem

Traditional request-response cycles create poor user experience:
- Users wait for server responses before seeing feedback
- Network delays make interfaces feel sluggish
- Failed operations require complete UI state rollback
- Loading states interrupt user workflow
- User confidence decreases with slow responses

## Solution

Optimistic UI Updates immediately reflect user actions in the interface before server confirmation:
- **Immediate Feedback**: Update UI instantly on user action
- **Temporary State**: Create provisional data while waiting for server
- **Rollback Strategy**: Revert changes if server operation fails
- **Eventual Consistency**: Replace temporary data with server response

## Implementation

### Basic Optimistic Update Pattern

```typescript
export function useOptimisticTaskCreation() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  
  return useMutation({
    mutationFn: tasksApi.create,
    
    onMutate: async (newTask) => {
      // Cancel any outgoing refetches
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.tasksList() 
      });
      
      // Create temporary task with optimistic ID
      const tempId = `temp-${Date.now()}`;
      const optimisticTask = {
        ...newTask,
        id: tempId,
        status: 'creating' as const,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };
      
      // Snapshot the previous value
      const previousTasks = queryClient.getQueryData(
        queryKeys.tasksList()
      );
      
      // Optimistically update to the new value
      queryClient.setQueryData(
        queryKeys.tasksList(),
        (old: Task[] = []) => [...old, optimisticTask]
      );
      
      // Also set the individual task data
      queryClient.setQueryData(
        queryKeys.taskDetail(tempId),
        optimisticTask
      );
      
      // Navigate immediately to the temporary task
      navigate(`/tasks/${tempId}`);
      
      // Return a context object with the snapshotted value
      return { previousTasks, tempId, optimisticTask };
    },
    
    onError: (err, newTask, context) => {
      // If the mutation fails, use the context returned from onMutate
      queryClient.setQueryData(
        queryKeys.tasksList(),
        context?.previousTasks
      );
      
      // Remove the temporary task
      if (context?.tempId) {
        queryClient.removeQueries({
          queryKey: queryKeys.taskDetail(context.tempId),
        });
      }
      
      // Navigate back to tasks list
      navigate('/tasks');
      
      // Show error notification
      toast.error('Failed to create task');
    },
    
    onSuccess: (data, variables, context) => {
      // Replace temporary data with real data
      if (context?.tempId) {
        // Remove temp task from cache
        queryClient.removeQueries({
          queryKey: queryKeys.taskDetail(context.tempId),
        });
        
        // Set real task data
        queryClient.setQueryData(
          queryKeys.taskDetail(data.id),
          data
        );
        
        // Update URL to real ID
        navigate(`/tasks/${data.id}`, { replace: true });
      }
      
      // Invalidate and refetch tasks list to ensure consistency
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      });
    },
  });
}
```

### Complex Optimistic Updates with Relationships

```typescript
export function useOptimisticTaskUpdate() {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn: ({ taskId, updates }: { taskId: string; updates: Partial<Task> }),
    
    onMutate: async ({ taskId, updates }) => {
      // Cancel related queries
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.task(taskId) 
      });
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.tasksList() 
      });
      
      // Snapshot current state
      const previousTask = queryClient.getQueryData(
        queryKeys.taskDetail(taskId)
      );
      const previousTasksList = queryClient.getQueryData(
        queryKeys.tasksList()
      );
      
      // Update task detail
      queryClient.setQueryData(
        queryKeys.taskDetail(taskId),
        (old: Task) => old ? { ...old, ...updates } : old
      );
      
      // Update task in list
      queryClient.setQueryData(
        queryKeys.tasksList(),
        (old: Task[] = []) => 
          old.map(task => 
            task.id === taskId 
              ? { ...task, ...updates }
              : task
          )
      );
      
      return { previousTask, previousTasksList };
    },
    
    onError: (err, { taskId }, context) => {
      // Rollback all changes
      if (context?.previousTask) {
        queryClient.setQueryData(
          queryKeys.taskDetail(taskId),
          context.previousTask
        );
      }
      
      if (context?.previousTasksList) {
        queryClient.setQueryData(
          queryKeys.tasksList(),
          context.previousTasksList
        );
      }
    },
    
    onSettled: (data, error, { taskId }) => {
      // Always refetch to ensure server state is correct
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      });
    },
  });
}
```

### Optimistic List Operations

```typescript
export function useOptimisticTaskDeletion() {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn: tasksApi.delete,
    
    onMutate: async (taskId) => {
      await queryClient.cancelQueries({ 
        queryKey: queryKeys.tasksList() 
      });
      
      const previousTasks = queryClient.getQueryData(
        queryKeys.tasksList()
      );
      
      // Remove task from list immediately
      queryClient.setQueryData(
        queryKeys.tasksList(),
        (old: Task[] = []) => old.filter(task => task.id !== taskId)
      );
      
      // Mark task as deleted in detail view
      queryClient.setQueryData(
        queryKeys.taskDetail(taskId),
        (old: Task) => old ? { ...old, status: 'deleting' } : old
      );
      
      return { previousTasks };
    },
    
    onError: (err, taskId, context) => {
      // Restore the list
      queryClient.setQueryData(
        queryKeys.tasksList(),
        context?.previousTasks
      );
      
      // Restore task detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskDetail(taskId),
      });
    },
    
    onSuccess: (data, taskId) => {
      // Remove task data completely
      queryClient.removeQueries({
        queryKey: queryKeys.taskDetail(taskId),
      });
    },
  });
}
```

## Advanced Patterns

### Optimistic Updates with Validation

```typescript
function useOptimisticWithValidation<T, U>(
  mutationFn: (data: T) => Promise<U>,
  options: {
    validate?: (data: T) => boolean;
    optimisticData: (input: T) => U;
    queryKey: QueryKey;
  }
) {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn,
    
    onMutate: async (data) => {
      // Validate before applying optimistic update
      if (options.validate && !options.validate(data)) {
        throw new Error('Validation failed');
      }
      
      await queryClient.cancelQueries({ queryKey: options.queryKey });
      
      const previousData = queryClient.getQueryData(options.queryKey);
      
      // Apply optimistic update
      queryClient.setQueryData(
        options.queryKey,
        options.optimisticData(data)
      );
      
      return { previousData };
    },
    
    onError: (err, data, context) => {
      if (context?.previousData !== undefined) {
        queryClient.setQueryData(options.queryKey, context.previousData);
      }
    },
  });
}
```

### Batch Optimistic Operations

```typescript
export function useOptimisticBatchUpdate() {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn: async (operations: BatchOperation[]) => {
      return tasksApi.batchUpdate(operations);
    },
    
    onMutate: async (operations) => {
      const snapshots = new Map();
      
      // Apply all optimistic updates
      for (const op of operations) {
        await queryClient.cancelQueries({
          queryKey: queryKeys.taskDetail(op.taskId),
        });
        
        const previous = queryClient.getQueryData(
          queryKeys.taskDetail(op.taskId)
        );
        snapshots.set(op.taskId, previous);
        
        queryClient.setQueryData(
          queryKeys.taskDetail(op.taskId),
          (old: Task) => ({ ...old, ...op.updates })
        );
      }
      
      return { snapshots };
    },
    
    onError: (err, operations, context) => {
      // Rollback all operations
      if (context?.snapshots) {
        for (const [taskId, previousData] of context.snapshots) {
          queryClient.setQueryData(
            queryKeys.taskDetail(taskId),
            previousData
          );
        }
      }
    },
    
    onSettled: (data, error, operations) => {
      // Refetch all affected tasks
      operations.forEach(op => {
        queryClient.invalidateQueries({
          queryKey: queryKeys.taskDetail(op.taskId),
        });
      });
    },
  });
}
```

### Optimistic Navigation

```typescript
export function useOptimisticNavigation() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  
  const navigateOptimistically = useCallback(
    (path: string, optimisticData?: any) => {
      // Navigate immediately
      navigate(path);
      
      // If we have optimistic data, set it in cache
      if (optimisticData) {
        const queryKey = pathToQueryKey(path);
        queryClient.setQueryData(queryKey, optimisticData);
      }
    },
    [navigate, queryClient]
  );
  
  const rollbackNavigation = useCallback(
    (fallbackPath: string) => {
      navigate(fallbackPath, { replace: true });
    },
    [navigate]
  );
  
  return { navigateOptimistically, rollbackNavigation };
}
```

## UI Patterns

### Loading States with Optimistic Updates

```typescript
function TaskCard({ task }: { task: Task }) {
  const updateTask = useOptimisticTaskUpdate();
  const isOptimistic = task.id.startsWith('temp-');
  const isUpdating = updateTask.isPending;
  
  return (
    <Card className={cn(
      'task-card',
      isOptimistic && 'opacity-70 animate-pulse',
      isUpdating && 'pointer-events-none'
    )}>
      <div className="flex items-center gap-2">
        <h3>{task.title}</h3>
        {isOptimistic && (
          <Badge variant="secondary" size="sm">
            Creating...
          </Badge>
        )}
        {isUpdating && (
          <Spinner size="sm" />
        )}
      </div>
      
      <p>{task.description}</p>
      
      <Button
        onClick={() => updateTask.mutate({
          taskId: task.id,
          updates: { status: 'completed' }
        })}
        disabled={isOptimistic || isUpdating}
      >
        Mark Complete
      </Button>
    </Card>
  );
}
```

### Error Recovery UI

```typescript
function OptimisticErrorBoundary({ 
  children, 
  onRetry 
}: { 
  children: React.ReactNode;
  onRetry: () => void;
}) {
  return (
    <ErrorBoundary
      fallback={({ error, reset }) => (
        <Alert variant="destructive">
          <AlertTriangle className="h-4 w-4" />
          <AlertTitle>Operation Failed</AlertTitle>
          <AlertDescription>
            {error.message}
          </AlertDescription>
          <div className="flex gap-2 mt-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                reset();
                onRetry();
              }}
            >
              Retry
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={reset}
            >
              Dismiss
            </Button>
          </div>
        </Alert>
      )}
    >
      {children}
    </ErrorBoundary>
  );
}
```

## Benefits

### User Experience
- **Instant Feedback**: Users see immediate response to actions
- **Perceived Performance**: Application feels faster and more responsive
- **Reduced Waiting**: No loading states for simple operations
- **Confidence**: Users trust their actions are being processed

### Performance
- **Reduced Round Trips**: Less waiting for server responses
- **Better Caching**: Optimistic data can be cached and reused
- **Smoother Interactions**: No jarring loading states

## Trade-offs

### Complexity
- More complex state management
- Error handling becomes more intricate
- Need to handle rollback scenarios
- Testing optimistic flows is harder

### Consistency Risks
- Temporary inconsistency with server state
- Potential race conditions
- Need to handle conflicts carefully

### User Confusion
- Failed operations might confuse users
- Rollbacks can be jarring if not handled well
- Need clear visual indicators for optimistic state

## Best Practices

### Do's
- **Always provide rollback** for failed operations
- **Use visual indicators** to show optimistic state
- **Test failure scenarios** thoroughly
- **Keep optimistic updates simple** when possible
- **Validate data** before optimistic updates
- **Use timeouts** for long-running operations

### Don'ts
- Don't make optimistic updates for critical operations
- Don't forget to handle race conditions
- Don't ignore server validation errors
- Don't make irreversible optimistic changes
- Don't skip error messaging

### Visual Guidelines
```css
/* Optimistic state indicators */
.optimistic {
  opacity: 0.7;
  animation: pulse 2s infinite;
}

.optimistic-success {
  background-color: var(--success-50);
  border: 1px solid var(--success-200);
}

.optimistic-error {
  background-color: var(--error-50);
  border: 1px solid var(--error-200);
  animation: shake 0.5s ease-in-out;
}

@keyframes shake {
  0%, 100% { transform: translateX(0); }
  25% { transform: translateX(-4px); }
  75% { transform: translateX(4px); }
}
```

## Testing Strategies

### Unit Testing Optimistic Updates

```typescript
describe('OptimisticTaskCreation', () => {
  it('should create optimistic task immediately', async () => {
    const { result } = renderHook(() => useOptimisticTaskCreation(), {
      wrapper: QueryWrapper,
    });
    
    act(() => {
      result.current.mutate({
        title: 'Test Task',
        description: 'Test Description',
      });
    });
    
    // Check optimistic update happened immediately
    const tasks = queryClient.getQueryData(queryKeys.tasksList());
    expect(tasks).toContainEqual(
      expect.objectContaining({
        title: 'Test Task',
        status: 'creating',
      })
    );
  });
  
  it('should rollback on error', async () => {
    // Mock API to fail
    tasksApi.create.mockRejectedValueOnce(new Error('Network error'));
    
    const { result } = renderHook(() => useOptimisticTaskCreation());
    
    act(() => {
      result.current.mutate({ title: 'Test Task' });
    });
    
    await waitFor(() => {
      expect(result.current.isError).toBe(true);
    });
    
    // Check rollback happened
    const tasks = queryClient.getQueryData(queryKeys.tasksList());
    expect(tasks).not.toContainEqual(
      expect.objectContaining({ title: 'Test Task' })
    );
  });
});
```

## Related Patterns
- [Smart Cache Invalidation](./cache-invalidation.md) - Handles cache updates
- [React Query Patterns](./react-query.md) - Provides the mutation framework
- [Error Handling Patterns](./error-handling.md) - Manages rollback scenarios

## Further Reading
- [Optimistic UI Patterns](https://uxdesign.cc/optimistic-ui-patterns-for-improved-perceived-performance-6b044f4e9d8d)
- [React Query Optimistic Updates](https://tanstack.com/query/latest/docs/react/guides/optimistic-updates)
- [The UX of Optimistic UI](https://www.smashingmagazine.com/2016/11/true-lies-of-optimistic-user-interfaces/)
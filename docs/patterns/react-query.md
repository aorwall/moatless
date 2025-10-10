# React Query Patterns

## Implementation

### Query Key Architecture

```typescript
// Hierarchical query key factory
export const queryKeys = {
  // Top level - invalidates everything
  tasks: () => ['tasks'] as const,
  
  // Specific task - invalidates all data for this task
  task: (taskId: string) => [...queryKeys.tasks(), taskId] as const,
  
  // Task data types - granular invalidation
  taskDetail: (taskId: string) => [...queryKeys.task(taskId), 'detail'] as const,
  taskEvents: (taskId: string) => [...queryKeys.task(taskId), 'events'] as const,
  taskGitDiff: (taskId: string) => [...queryKeys.task(taskId), 'git-diff'] as const,
  workspace: (workspaceId?: string) => ['workspaces', workspaceId] as const,
  
  // With filters - precise cache entries
  tasksList: (filters?: TaskFilters) => [...queryKeys.tasks(), 'list', filters] as const,
};
```

### Query Hooks Pattern

```typescript
// Consistent hook structure
export function useTasksList(options?: UseTasksListOptions) {
  return useQuery({
    queryKey: queryKeys.tasksList(options?.filters),
    queryFn: () => tasksApi.list(options?.filters),
    // Override global defaults only when the hook needs different behaviour
    // (global defaults live in apps/frontend/src/lib/api/query-client.ts)
    ...options?.queryOptions,
  });
}
```

### Task Detail Consumption

```typescript
// Inside components wrapped by TaskProviders
const taskId = useTaskId();
const { data: task, isLoading, error } = useTaskDetailQuery(taskId);

// Workspace polling only runs while the task is open
const workspaceStatusQuery = useWorkspaceStatus(taskId, task);
const gitDiffQuery = useTaskGitDiffQuery(taskId, { retry: false });

// Streaming state comes from context buffers
const { streamingEvents, isStreaming } = useTaskStreamingState();
```

> **Note:** `TaskDataProvider` exposes only `taskId` and SSE buffers. Components opt into granular React Query hooks so unrelated sections avoid rerenders and cache lifetimes remain predictable.

### Mutation Hooks Pattern

```typescript
export function useCreateTask(options?: UseCreateTaskOptions) {
  const queryClient = useQueryClient();

  // Destructure handlers to compose them explicitly
  const { onError, onSuccess, ...rest } = options || {}

  return useMutation({
    ...rest,
    mutationFn: tasksApi.create,

    // Optimistic update
    onMutate: async (newTask) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks() });

      const previousTasks = queryClient.getQueryData(queryKeys.tasksList());

      return { previousTasks };
    },

    // Handle errors - internal logic then component handler
    onError: (err, newTask, context) => {
      if (context?.previousTasks) {
        queryClient.setQueryData(queryKeys.tasksList(), context.previousTasks);
      }
      onError?.(err, newTask, context);
    },

    // Update with real data - internal logic then component handler
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.tasksList(), (old) => {
        return [...(old || []), data];
      });

      queryClient.setQueryData(queryKeys.taskDetail(data.id), data);
      queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() });
      onSuccess?.(data);
    },
  });
}
```

**Pattern**: When hooks need internal lifecycle logic (rollback, cache updates), destructure handlers from options and compose them explicitly. This ensures internal logic runs before component callbacks. For simple hooks without internal handlers, just spread `...options`. Always type API results with helpers such as `RouteSuccess<typeof route>` so query caches stay aligned with the OpenAPI contract.

### Mutation Error Handling Pattern

All mutations should provide user feedback when they fail. Follow the canonical `useApiError` guidance in `docs/patterns/api-best-practices.md` so toast content, validation messages, and fallbacks remain consistent across the app.

### Smart Invalidation

```typescript
// Event-driven invalidation
export function useEventDrivenInvalidation() {
  const queryClient = useQueryClient();
  
  useEffect(() => {
    const unsubscribe = eventBus.subscribe((event) => {
      switch (event.type) {
        case 'TASK_STATUS_CHANGED':
        case 'TASK_COMPLETED':
        case 'TASK_FAILED':
          // Invalidate specific task and lists
          queryClient.invalidateQueries({
            queryKey: queryKeys.task(event.taskId),
          });
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasksList(),
          });
          break;
          
        case 'TASK_EVENT_ADDED':
          // Only invalidate events, not entire task
          queryClient.invalidateQueries({
            queryKey: queryKeys.taskEvents(event.taskId),
          });
          break;
      }
    });
    
    return unsubscribe;
  }, [queryClient]);
}
```

> **Note:** High-frequency events can cause thrashing if every emission triggers an invalidation. Prefer debouncing, throttling, or targeted `setQueryData` updates when the event stream is noisy.

## Patterns in Practice

### 1. Prefetching

```typescript
// Prefetch on hover for instant navigation
function TaskListItem({ task }: { task: Task }) {
  const queryClient = useQueryClient();
  
  const prefetchTask = () => {
    queryClient.prefetchQuery({
      queryKey: queryKeys.taskDetail(task.id),
      queryFn: () => tasksApi.get(task.id),
      staleTime: 10 * 1000, // 10 seconds
    });
  };
  
  return (
    <Link 
      to={`/tasks/${task.id}`}
      onMouseEnter={prefetchTask}
    >
      {task.title}
    </Link>
  );
}
```

### 2. Dependent Queries

```typescript
// Query depends on another query's result
function useTaskWithWorkspace(taskId: string) {
  const { data: task } = useTask(taskId);

  const workspaceId = task?.workspaceId;

  const { data: workspace } = useQuery({
    queryKey: queryKeys.workspace(workspaceId),
    queryFn: () => workspaceApi.get(workspaceId!),
    enabled: !!workspaceId, // Only run when we have workspaceId
  });

  return { task, workspace };
}
```

### 3. Infinite Queries

```typescript
// For paginated data
export function useInfiniteTaskEvents(taskId: string) {
  return useInfiniteQuery({
    queryKey: queryKeys.taskEvents(taskId),
    queryFn: ({ pageParam = 0 }) => 
      eventsApi.list(taskId, { offset: pageParam, limit: 50 }),
    getNextPageParam: (lastPage, pages) => 
      lastPage.hasMore ? pages.length * 50 : undefined,
    staleTime: 0, // Always fresh for real-time data
  });
}
```

### 4. Optimistic Updates with Navigation

```typescript
export function useCreateTaskWithNavigation() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: tasksApi.create,

    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.taskDetail(data.id), data);
      navigate(`/tasks/${data.id}`);
    },
  });
}
```

## Best Practices

### DO's
- Use query key factories for consistency
- Implement optimistic updates for better UX
- Set appropriate `staleTime` based on data volatility
- Use `enabled` option for dependent queries
- Prefetch data for predictable user actions
- Handle mutation errors with toast notifications using the shared pattern in `docs/patterns/api-best-practices.md`

### DON'Ts
- Don't store server state in React state
- Don't manually refetch unless necessary
- Don't ignore error states or mutation failures
- Don't create query keys manually
- Don't mutate cache data directly
- **Don't show raw error messages to users** - format them with `useApiError`

## Configuration

### Global Query Client Setup

```typescript
// apps/frontend/src/lib/api/query-client.ts
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5 * 60 * 1000,     // 5 minutes
      gcTime: 10 * 60 * 1000,       // 10 minutes  
      retry: 3,
      retryDelay: attemptIndex => 
        Math.min(1000 * 2 ** attemptIndex, 30000),
      refetchOnWindowFocus: false,
      refetchOnReconnect: 'always',
    },
    mutations: {
      retry: 1,
      retryDelay: 1000,
    },
  },
});
```

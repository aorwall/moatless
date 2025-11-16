# React Query Patterns

## Implementation

### Query Key Architecture

The project uses a nested object structure for query keys, providing better organization and IDE autocomplete:

```typescript
// Hierarchical query key factory with nested structure
export const queryKeys = {
  all: ['swarm'] as const,

  // Nested task structure with methods
  tasks: {
    all: ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all, 'list'] as const,
    list: (filters?: { activeOnly?: boolean; limit?: number }) =>
      [...queryKeys.tasks.lists(), filters] as const,

    // Task detail and sub-resources
    detail: (taskId: string) => [...queryKeys.tasks.all, 'detail', taskId] as const,
    messages: (taskId: string, filters?: { limit?: number; offset?: number }) =>
      [...queryKeys.tasks.detail(taskId), 'messages', filters] as const,
    gitDiff: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'git-diff'] as const,
    workspaceStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'workspace'] as const,

    // File operations under dedicated namespace
    files: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'files'] as const,
    fileList: (taskId: string, path?: string) =>
      [...queryKeys.tasks.files(taskId), 'list', { path }] as const,
    fileContent: (taskId: string, filePath: string) =>
      [...queryKeys.tasks.files(taskId), 'content', filePath] as const,
  },

  // Other top-level namespaces
  repositories: () => [...queryKeys.all, 'repositories'] as const,
  repositoriesList: () => [...queryKeys.repositories(), 'list'] as const,
  repositoryDetail: (repoId: string) => [...queryKeys.repositories(), 'detail', repoId] as const,

  settings: {
    all: () => [...queryKeys.all, 'settings'] as const,
    detail: () => [...queryKeys.settings.all(), 'detail'] as const,
    claude: () => [...queryKeys.settings.all(), 'claude'] as const,
    github: () => [...queryKeys.settings.all(), 'github'] as const,
  },
} as const;
```

**Benefits of nested structure:**
- Better IDE autocomplete (`queryKeys.tasks.detail(id)` instead of `queryKeys.taskDetail(id)`)
- Clearer hierarchy and relationships
- Easier to find related keys
- Automatic grouping for invalidation

### Query Hooks Pattern

```typescript
// Consistent hook structure using nested query keys
export function useTasksList(options?: UseTasksListOptions) {
  return useQuery({
    queryKey: queryKeys.tasks.list(options?.filters),
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
const { data: task, isLoading, error } = useQuery({
  queryKey: queryKeys.tasks.detail(taskId),
  queryFn: () => tasksApi.get(taskId),
});

// Sub-resource queries - automatically invalidated when parent task invalidates
const { data: gitDiff } = useQuery({
  queryKey: queryKeys.tasks.gitDiff(taskId),
  queryFn: () => tasksApi.getGitDiff(taskId),
  retry: false, // Override global default for specific behavior
});

const { data: workspaceStatus } = useQuery({
  queryKey: queryKeys.tasks.workspaceStatus(taskId),
  queryFn: () => tasksApi.getWorkspaceStatus(taskId),
  enabled: !!task, // Only fetch when task exists
});

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
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.all });

      const previousTasks = queryClient.getQueryData(queryKeys.tasks.list());

      return { previousTasks };
    },

    // Handle errors - internal logic then component handler
    onError: (err, newTask, context) => {
      if (context?.previousTasks) {
        queryClient.setQueryData(queryKeys.tasks.list(), context.previousTasks);
      }
      onError?.(err, newTask, context);
    },

    // Update with real data - internal logic then component handler
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.tasks.list(), (old) => {
        return [...(old || []), data];
      });

      queryClient.setQueryData(queryKeys.tasks.detail(data.id), data);
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() });
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
// Event-driven invalidation with nested query keys
export function useEventDrivenInvalidation() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const unsubscribe = eventBus.subscribe((event) => {
      switch (event.type) {
        case 'TASK_STATUS_CHANGED':
        case 'TASK_COMPLETED':
        case 'TASK_FAILED':
          // Invalidate specific task detail (invalidates all sub-resources)
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasks.detail(event.taskId),
          });
          // Invalidate task lists
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasks.lists(),
          });
          break;

        case 'TASK_MESSAGE_ADDED':
          // Only invalidate messages, not entire task
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasks.messages(event.taskId),
          });
          break;

        case 'FILE_CHANGED':
          // Only invalidate file-related queries
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasks.files(event.taskId),
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
      queryKey: queryKeys.tasks.detail(task.id),
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
  const { data: task } = useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.get(taskId),
  });

  const { data: workspaceStatus } = useQuery({
    queryKey: queryKeys.tasks.workspaceStatus(taskId),
    queryFn: () => tasksApi.getWorkspaceStatus(taskId),
    enabled: !!task, // Only run when we have task data
  });

  return { task, workspaceStatus };
}
```

### 3. Optimistic Updates with Navigation

```typescript
export function useCreateTaskWithNavigation() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: tasksApi.create,

    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.tasks.detail(data.id), data);
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
      staleTime: 1000 * 60 * 5,     // 5 minutes
      gcTime: 1000 * 60 * 10,       // 10 minutes
      retry: false,                 // Disabled globally, enable per-query if needed
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: false,                 // Disabled globally, enable per-mutation if needed
    },
  },
});
```

**Configuration notes:**
- **Retry disabled globally**: Most queries don't need automatic retries. Enable per-query for critical data.
- **No window focus refetch**: Prevents unnecessary requests when switching browser tabs.
- **5-minute stale time**: Cached data is considered fresh for 5 minutes.
- **10-minute garbage collection**: Unused cache entries are removed after 10 minutes.

# Data Fetching with TanStack Query

## Table of Contents
- [Overview](#overview)
- [Query Setup](#query-setup)
- [Creating Query Hooks](#creating-query-hooks)
- [Mutation Hooks](#mutation-hooks)
- [Query Keys](#query-keys)
- [Cache Invalidation](#cache-invalidation)
- [Error Handling](#error-handling)
- [Optimistic Updates](#optimistic-updates)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

**TanStack Query (React Query)** is used for ALL server state management in this project.

### Key Principles
- Use TanStack Query for all server state
- Never duplicate server state in React state
- Use centralized query keys from `src/lib/api/query-keys.ts`
- Always call the API through the generated `@moatless/api-client` fetchers
- Handle loading, error, and empty states

### Technology Stack
- `@tanstack/react-query` - Server state management
- `@moatless/api-client` - Orval-generated types and typed fetchers (regenerate with `bun run generate:api`)
- `unwrapResponse` from `@moatless/api-client/custom-instance` - unwraps the response body, throws on error status

## Query Setup

### Query Client Configuration
Located in `src/lib/api/query-client.ts`:

```tsx
import { QueryClient } from '@tanstack/react-query'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,  // Don't retry by default
      refetchOnWindowFocus: false,
      staleTime: 1000 * 60 * 5,  // 5 minutes
    },
    mutations: {
      retry: false,
    }
  }
})
```

### Provider Setup
In `src/main.tsx`:

```tsx
import { QueryClientProvider } from '@tanstack/react-query'
import { queryClient } from '@/lib/api/query-client'

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      {/* Your app */}
    </QueryClientProvider>
  )
}
```

## Creating Query Hooks

### Basic Query Hook Pattern
Create query hooks in feature's `api/` directory:

```tsx
// src/features/tasks/api/use-tasks-query.ts
import { useQuery } from '@tanstack/react-query'
import { listTasks } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'

export function useTasksQuery() {
  return useQuery({
    queryKey: queryKeys.tasks.list(),
    queryFn: () => unwrapResponse(listTasks())
  })
}
```

### Query Hook with Parameters
```tsx
// src/features/tasks/api/use-task-query.ts
import { useQuery } from '@tanstack/react-query'
import { getTask } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'

export function useTaskQuery(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => unwrapResponse(getTask(taskId)),
    enabled: !!taskId  // Only run if taskId exists
  })
}
```

### Using Queries in Components
```tsx
import { useTasksQuery } from '@/features/tasks/api/use-tasks-query'

export function TaskList() {
  const { data: tasks, isLoading, error } = useTasksQuery()

  if (isLoading) {
    return <LoadingSpinner />
  }

  if (error) {
    return <ErrorDisplay error={error} />
  }

  if (!tasks || tasks.length === 0) {
    return <EmptyState message="No tasks found" />
  }

  return (
    <div className="space-y-4">
      {tasks.map(task => (
        <TaskCard key={task.id} {...task} />
      ))}
    </div>
  )
}
```

## Mutation Hooks

### Basic Mutation Pattern
```tsx
// src/features/tasks/api/use-create-task-mutation.ts
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createTask } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'
import type { CreateTaskRequest } from '@moatless/api-client'

export function useCreateTaskMutation() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: CreateTaskRequest) => unwrapResponse(createTask(data)),
    onSuccess: () => {
      // Invalidate tasks list to refetch
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.list()
      })
    }
  })
}
```

### Using Mutations in Components
```tsx
import { useCreateTaskMutation } from '@/features/tasks/api/use-create-task-mutation'
import { useApiError } from '@/lib/hooks/use-api-error'

export function CreateTaskForm() {
  const [title, setTitle] = useState('')
  const createTask = useCreateTaskMutation()
  const { showError } = useApiError()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    try {
      await createTask.mutateAsync({ title })
      setTitle('')  // Clear form on success
      toast.success('Task created!')
    } catch (error) {
      showError(error)
    }
  }

  return (
    <form onSubmit={handleSubmit}>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="Task title"
      />
      <button
        type="submit"
        disabled={createTask.isPending}
      >
        {createTask.isPending ? 'Creating...' : 'Create Task'}
      </button>
    </form>
  )
}
```

### Update Mutation Pattern
```tsx
// src/features/tasks/api/use-update-task-mutation.ts
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { updateTaskMetadata } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'
import type { UpdateTaskMetadataRequest } from '@moatless/api-client'

export function useUpdateTaskMutation(taskId: string) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: UpdateTaskMetadataRequest) => unwrapResponse(updateTaskMetadata(taskId, data)),
    onSuccess: (updatedTask) => {
      // Update the specific task in cache
      queryClient.setQueryData(
        queryKeys.tasks.detail(taskId),
        updatedTask
      )

      // Invalidate list to refetch
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.list()
      })
    }
  })
}
```

### Delete Mutation Pattern
```tsx
// src/features/tasks/api/use-delete-task-mutation.ts
export function useDeleteTaskMutation() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => unwrapResponse(deleteTask(taskId)),
    onSuccess: (_, taskId) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.tasks.detail(taskId)
      })

      // Invalidate list
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.list()
      })
    }
  })
}
```

## Query Keys

### Centralized Query Keys
All query keys are defined in `src/lib/api/query-keys.ts`:

```tsx
// src/lib/api/query-keys.ts
export const queryKeys = {
  tasks: {
    all: () => ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all(), 'list'] as const,
    list: (filters?: TaskFilters) =>
      [...queryKeys.tasks.lists(), filters] as const,
    details: () => [...queryKeys.tasks.all(), 'detail'] as const,
    detail: (id: string) => [...queryKeys.tasks.details(), id] as const
  },
  environments: {
    all: () => ['environments'] as const,
    lists: () => [...queryKeys.environments.all(), 'list'] as const,
    list: () => [...queryKeys.environments.lists()] as const,
    details: () => [...queryKeys.environments.all(), 'detail'] as const,
    detail: (id: string) =>
      [...queryKeys.environments.details(), id] as const
  }
}
```

### Query Key Structure
Hierarchical structure for efficient invalidation:

```
['tasks']                          # All task queries
  ['tasks', 'list']                # All list queries
    ['tasks', 'list', {...}]       # Specific list query
  ['tasks', 'detail']              # All detail queries
    ['tasks', 'detail', 'task-1']  # Specific task detail
```

### Benefits of Centralized Keys
1. **Type Safety**: TypeScript autocomplete and validation
2. **Consistency**: Same keys used everywhere
3. **Easy Invalidation**: Hierarchical structure
4. **Refactoring**: Change keys in one place

## Cache Invalidation

### Invalidate Specific Query
```tsx
// Invalidate a specific task detail
queryClient.invalidateQueries({
  queryKey: queryKeys.tasks.detail(taskId)
})
```

### Invalidate All Related Queries
```tsx
// Invalidate all task queries (list + details)
queryClient.invalidateQueries({
  queryKey: queryKeys.tasks.all()
})
```

### Invalidate List Only
```tsx
// Invalidate only task lists
queryClient.invalidateQueries({
  queryKey: queryKeys.tasks.lists()
})
```

### Event-Driven Invalidation
Use the event system for cross-feature cache invalidation:

```tsx
import { eventBus } from '@/lib/events/event-bus'

// Emit event after mutation
eventBus.emit('task:updated', { taskId })

// Listen and invalidate in another component
useEffect(() => {
  const unsubscribe = eventBus.on('task:updated', ({ taskId }) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.detail(taskId)
    })
  })

  return unsubscribe
}, [])
```

## Error Handling

### Using useApiError Hook
```tsx
import { useApiError } from '@/lib/hooks/use-api-error'

export function TaskForm() {
  const createTask = useCreateTaskMutation()
  const { showError } = useApiError()

  const handleSubmit = async (data: CreateTaskRequest) => {
    try {
      await createTask.mutateAsync(data)
      toast.success('Task created!')
    } catch (error) {
      showError(error)  // Displays user-friendly error
    }
  }
}
```

### Error Display Component
```tsx
import { ApiError } from '@/lib/api/client'

interface ErrorDisplayProps {
  error: unknown
  retry?: () => void
}

export function ErrorDisplay({ error, retry }: ErrorDisplayProps) {
  const message = error instanceof ApiError
    ? error.message
    : 'An unexpected error occurred'

  return (
    <div className="border border-destructive bg-destructive/10 p-4 rounded">
      <p className="text-destructive">{message}</p>
      {retry && (
        <button onClick={retry} className="mt-2 text-destructive">
          Try Again
        </button>
      )}
    </div>
  )
}
```

### Handling Specific Errors
```tsx
import { ApiError } from '@/lib/api/client'

try {
  await createTask.mutateAsync(data)
} catch (error) {
  if (error instanceof ApiError) {
    if (error.statusCode === 409) {
      toast.error('Task with this name already exists')
    } else if (error.statusCode === 403) {
      toast.error('You do not have permission')
    } else {
      toast.error(error.message)
    }
  } else {
    toast.error('An unexpected error occurred')
  }
}
```

## Optimistic Updates

### Optimistic Create
```tsx
export function useCreateTaskMutation() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: CreateTaskRequest) => unwrapResponse(createTask(data)),
    onMutate: async (newTask) => {
      // Cancel outgoing refetches
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.list()
      })

      // Snapshot previous value
      const previousTasks = queryClient.getQueryData(
        queryKeys.tasks.list()
      )

      // Optimistically update cache
      queryClient.setQueryData(
        queryKeys.tasks.list(),
        (old: TaskResponse[] = []) => [...old, { ...newTask, id: 'temp-id' }]
      )

      return { previousTasks }
    },
    onError: (err, newTask, context) => {
      // Rollback on error
      if (context?.previousTasks) {
        queryClient.setQueryData(
          queryKeys.tasks.list(),
          context.previousTasks
        )
      }
    },
    onSettled: () => {
      // Refetch after mutation
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.list()
      })
    }
  })
}
```

### Optimistic Update
```tsx
export function useUpdateTaskMutation(taskId: string) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: UpdateTaskMetadataRequest) => unwrapResponse(updateTaskMetadata(taskId, data)),
    onMutate: async (updates) => {
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.detail(taskId)
      })

      const previousTask = queryClient.getQueryData(
        queryKeys.tasks.detail(taskId)
      )

      // Optimistically update
      queryClient.setQueryData(
        queryKeys.tasks.detail(taskId),
        (old: TaskResponse) => ({ ...old, ...updates })
      )

      return { previousTask }
    },
    onError: (err, updates, context) => {
      if (context?.previousTask) {
        queryClient.setQueryData(
          queryKeys.tasks.detail(taskId),
          context.previousTask
        )
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId)
      })
    }
  })
}
```

## Common Patterns

### Dependent Queries
Query B depends on data from Query A:

```tsx
function TaskWithAgent({ taskId }: { taskId: string }) {
  const { data: task } = useTaskQuery(taskId)
  const { data: agent } = useAgentQuery(task?.agentId, {
    enabled: !!task?.agentId  // Only run if agentId exists
  })

  return <div>{/* Render task and agent */}</div>
}
```

### Parallel Queries
Fetch multiple resources simultaneously:

```tsx
function Dashboard() {
  const tasksQuery = useTasksQuery()
  const environmentsQuery = useEnvironmentsQuery()
  const settingsQuery = useSettingsQuery()

  const isLoading = tasksQuery.isLoading ||
    environmentsQuery.isLoading ||
    settingsQuery.isLoading

  if (isLoading) return <Loading />

  return <div>{/* Render dashboard */}</div>
}
```

### Paginated Queries
```tsx
export function usePaginatedTasksQuery(page: number) {
  return useQuery({
    queryKey: queryKeys.tasks.list({ page }),
    queryFn: () => unwrapResponse(listTasks({ page })),
    keepPreviousData: true  // Keep old data while fetching new
  })
}
```

### Infinite Queries
```tsx
import { useInfiniteQuery } from '@tanstack/react-query'

export function useInfiniteTasksQuery() {
  return useInfiniteQuery({
    queryKey: queryKeys.tasks.lists(),
    queryFn: ({ pageParam = 1 }) => unwrapResponse(listTasks({ page: pageParam })),
    getNextPageParam: (lastPage, pages) => {
      return lastPage.hasMore ? pages.length + 1 : undefined
    }
  })
}
```

## Anti-Patterns

### ❌ Duplicating Server State
```tsx
// WRONG
function TaskList() {
  const { data: tasks } = useTasksQuery()
  const [localTasks, setLocalTasks] = useState(tasks)  // DON'T!

  return <div>{localTasks.map(...)}</div>
}

// CORRECT
function TaskList() {
  const { data: tasks } = useTasksQuery()
  return <div>{tasks.map(...)}</div>
}
```

### ❌ Hand-Rolling Fetch Calls
```tsx
// WRONG
const response = await fetch('/api/v1/tasks')

// CORRECT — use the generated client
import { listTasks } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
const tasks = await unwrapResponse(listTasks())
```

### ❌ Not Handling Loading/Error States
```tsx
// WRONG
function TaskList() {
  const { data } = useTasksQuery()
  return <div>{data.map(...)}</div>  // Crashes if data is undefined!
}

// CORRECT
function TaskList() {
  const { data, isLoading, error } = useTasksQuery()

  if (isLoading) return <Loading />
  if (error) return <Error error={error} />

  return <div>{data.map(...)}</div>
}
```

### ❌ Manual Query Key Strings
```tsx
// WRONG
queryClient.invalidateQueries({ queryKey: ['tasks'] })

// CORRECT
import { queryKeys } from '@/lib/api/query-keys'
queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() })
```

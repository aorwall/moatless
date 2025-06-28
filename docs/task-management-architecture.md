# Task Management Architecture

## Overview

This document outlines the desired architecture for the task management system in the Moatless Swarm UI. The architecture is designed to provide a robust, scalable, and maintainable solution for managing AI-powered tasks with real-time updates, proper state management, and seamless user experience.

## Core Principles

1. **Simple Lifecycle**: Clear and predictable task state transitions
2. **Optimistic UI Updates**: Immediate feedback using React Query's built-in features
3. **Real-time Synchronization**: SSE connections for live updates without complexity
4. **React Query First**: Leverage React Query's caching without custom abstractions
5. **Clean Architecture**: Separation of concerns without over-engineering

## Architecture Components

### 1. Task Lifecycle Management

#### Task Creation Flow
```typescript
// Simple task creation flow using React Query
1. User submits task request
2. Optimistic update via React Query's onMutate
3. Server creates task and returns full data
4. React Query updates cache with server response
5. Navigate to task detail with data already cached
```

#### Task States
```typescript
enum TaskStatus {
  PENDING = 'pending',      // Initial creation state
  OPEN = 'open',           // Ready for processing
  RUNNING = 'running',     // Currently being processed
  COMPLETED = 'completed', // Successfully finished
  FAILED = 'failed',      // Error occurred
  CANCELLED = 'cancelled', // User cancelled
}
```

### 2. Data Layer Architecture

#### React Query Usage
```typescript
// Leverage React Query's built-in cache management
// No custom cache manager needed - React Query handles:
// - Cache updates via setQueryData
// - Optimistic updates via onMutate
// - Cache invalidation via invalidateQueries
// - Background refetching and synchronization
```

#### Query Key Structure
```typescript
// Hierarchical query key structure
const queryKeys = {
  tasks: {
    all: ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all, 'list'] as const,
    list: (filters: TaskFilters) => [...queryKeys.tasks.lists(), filters] as const,
    details: () => [...queryKeys.tasks.all, 'detail'] as const,
    detail: (id: string) => [...queryKeys.tasks.details(), id] as const,
    
    // Sub-resources
    events: (id: string) => [...queryKeys.tasks.detail(id), 'events'] as const,
    iterations: (id: string) => [...queryKeys.tasks.detail(id), 'iterations'] as const,
    gitDiff: (id: string) => [...queryKeys.tasks.detail(id), 'git-diff'] as const,
  }
}
```

#### Cache Invalidation Strategy
```typescript
// Smart invalidation to keep UI in sync without over-fetching

// After task creation:
queryClient.invalidateQueries({ 
  queryKey: queryKeys.tasks.lists() // Only invalidate lists, not all tasks
})

// After task update (status change, etc):
queryClient.invalidateQueries({ 
  queryKey: queryKeys.tasks.detail(taskId) // Invalidate specific task and its sub-resources
})
// This also invalidates: events, iterations, gitDiff for this task

// After task deletion:
queryClient.removeQueries({ 
  queryKey: queryKeys.tasks.detail(taskId) // Remove from cache
})
queryClient.invalidateQueries({ 
  queryKey: queryKeys.tasks.lists() // Update lists
})

// Selective invalidation for performance:
// DON'T DO: queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all })
// This would refetch everything including all task details

// DO: Be specific about what needs updating
queryClient.invalidateQueries({ 
  queryKey: queryKeys.tasks.list({ status: 'open' }) // Only refetch open tasks list
})
```

### 3. Real-time Updates (SSE)

#### Simple SSE Hook
```typescript
// Simple hook for SSE connection management
function useTaskSSE(taskId: string) {
  const [events, setEvents] = useState<TaskEvent[]>([])
  const [isConnected, setIsConnected] = useState(false)
  
  useEffect(() => {
    const eventSource = new EventSource(`/api/tasks/${taskId}/events`)
    
    eventSource.onopen = () => setIsConnected(true)
    eventSource.onerror = () => setIsConnected(false)
    eventSource.onmessage = (event) => {
      const taskEvent = JSON.parse(event.data)
      setEvents(prev => [...prev, taskEvent])
      
      // Update React Query cache with new event
      queryClient.setQueryData(
        queryKeys.tasks.events(taskId),
        (old) => [...(old || []), taskEvent]
      )
    }
    
    return () => eventSource.close()
  }, [taskId])
  
  return { events, isConnected }
}
```

#### Event Synchronization
```typescript
// Simple event synchronization using React Query
function useTaskEvents(taskId: string) {
  // Fetch initial events via API
  const { data: apiEvents = [] } = useQuery({
    queryKey: queryKeys.tasks.events(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
  })
  
  // Stream new events via SSE
  const { events: sseEvents } = useTaskSSE(taskId)
  
  // Combine and deduplicate
  const allEvents = useMemo(() => {
    const eventMap = new Map()
    [...apiEvents, ...sseEvents].forEach(event => {
      eventMap.set(event.id, event)
    })
    return Array.from(eventMap.values())
      .sort((a, b) => a.timestamp - b.timestamp)
  }, [apiEvents, sseEvents])
  
  return allEvents
}
```

### 4. Data Flow & Timing

#### Task Creation Data Flow
```typescript
// Critical: Data must be available BEFORE navigation
1. User submits task → Create optimistic task with temp ID
2. Update React Query cache with optimistic data
3. Server creates task → Returns real task data
4. Update cache with real data (replace temp ID)
5. Navigate to task detail (data already in cache)
```

#### SSE Connection Timing
```typescript
// SSE should connect as soon as we have a real task ID
// Don't wait for other data to load first

interface SSEConnectionTiming {
  // Connect immediately when:
  hasTaskId: boolean // Real task ID available (not temp)
  
  // Don't wait for:
  hasMetadata: boolean // ❌ This causes delays
  hasEvents: boolean   // ❌ This causes missed events
  hasStatus: boolean   // ❌ This prevents early updates
}
```

#### Data Requirements by Component
```typescript
// What each component needs to render properly
const componentDataRequirements = {
  ChatView: {
    minimal: ['taskId'], // Can show loading state
    full: ['task', 'events', 'sseConnection'],
  },
  TaskHeader: {
    minimal: ['taskId', 'title'],
    full: ['task', 'status', 'environment'],
  },
  EventViewer: {
    minimal: ['taskId'],
    full: ['events', 'sseConnection'],
  },
}
```

### 5. State Management

#### UI State Store
```typescript
// Separate UI state from server state
interface TaskUIStore {
  // View states
  activeView: 'chat' | 'iterations' | 'events'
  sidebarOpen: boolean
  selectedIterationId: string | undefined
  
  // Filter states
  eventFilters: EventFilters
  messageFilters: MessageFilters
  
  // Actions
  setActiveView: (view: string) => void
  toggleSidebar: () => void
  selectIteration: (id: string | undefined) => void
  updateFilters: (filters: Partial<EventFilters>) => void
}
```

#### React Query Hooks
```typescript
// Simple, focused hooks using React Query

// Task creation with optimistic updates
export function useCreateTask() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  
  return useMutation({
    mutationFn: tasksApi.createTask,
    onMutate: async (request) => {
      // Cancel in-flight queries
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.all })
      
      // Create optimistic task
      const optimisticTask = {
        id: `temp-${Date.now()}`,
        title: request.message,
        status: 'pending',
        createdAt: new Date().toISOString(),
      }
      
      // Update cache immediately
      queryClient.setQueryData(
        queryKeys.tasks.detail(optimisticTask.id),
        optimisticTask
      )
      
      return { optimisticTask }
    },
    onSuccess: (data, variables, context) => {
      // Replace optimistic data with real data
      queryClient.setQueryData(
        queryKeys.tasks.detail(data.taskId),
        data
      )
      
      // Invalidate lists only (not all task details)
      queryClient.invalidateQueries({ 
        queryKey: queryKeys.tasks.lists() 
      })
      
      // Navigate with data already in cache
      navigate({ to: `/tasks/${data.taskId}` })
    },
  })
}

// Simple task data hook
export function useTask(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    // Data stays fresh for 5 minutes
    staleTime: 5 * 60 * 1000,
  })
}
```

### 6. Error Handling & Recovery

#### Simple Error Handling
```typescript
// Use React Query's built-in retry and error handling
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (failureCount, error) => {
        // Don't retry on 4xx errors
        if (error.status >= 400 && error.status < 500) {
          return false
        }
        // Retry up to 3 times for other errors
        return failureCount < 3
      },
      retryDelay: (attemptIndex) => {
        // Exponential backoff
        return Math.min(1000 * 2 ** attemptIndex, 30000)
      },
    },
  },
})

// Component-level error handling
function TaskDetail() {
  const { data, error, refetch } = useTask(taskId)
  
  if (error) {
    return (
      <ErrorAlert 
        error={error}
        onRetry={refetch}
      />
    )
  }
  
  // ... rest of component
}
```

### 7. Performance Optimizations

#### Parallel Data Fetching
```typescript
// Use React Query's parallel queries
function useTaskData(taskId: string) {
  const queries = useQueries({
    queries: [
      {
        queryKey: queryKeys.tasks.detail(taskId),
        queryFn: () => tasksApi.getTask(taskId),
      },
      {
        queryKey: queryKeys.tasks.events(taskId),
        queryFn: () => tasksApi.getTaskEvents(taskId),
      },
      {
        queryKey: queryKeys.tasks.iterations(taskId),
        queryFn: () => tasksApi.getTaskIterations(taskId),
      },
    ],
  })
  
  return {
    task: queries[0].data,
    events: queries[1].data,
    iterations: queries[2].data,
    isLoading: queries.some(q => q.isLoading),
    error: queries.find(q => q.error)?.error,
  }
}

// Prefetching for navigation
function prefetchTask(taskId: string) {
  return Promise.all([
    queryClient.prefetchQuery({
      queryKey: queryKeys.tasks.detail(taskId),
      queryFn: () => tasksApi.getTask(taskId),
    }),
    queryClient.prefetchQuery({
      queryKey: queryKeys.tasks.events(taskId),
      queryFn: () => tasksApi.getTaskEvents(taskId),
    }),
  ])
}
```

## Concrete Refactoring Plan

### Phase 1: Fix Task Creation Flow (Priority: Critical)

#### Current Problems:
- No optimistic updates in `useCreateTask`
- Navigation happens before data is cached
- ChatView renders without data

#### Refactoring Steps:

1. **Update `useCreateTask` hook** (`client/src/features/tasks/hooks/use-tasks.ts`):
```typescript
export function useCreateTask(options?: UseMutationOptions<SimpleTaskResponse, Error, CreateTaskRequest>) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  return useMutation({
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    
    // Add optimistic update
    onMutate: async (request) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks() })
      
      const optimisticTask = {
        id: `temp-${Date.now()}`,
        taskId: `temp-${Date.now()}`,
        title: request.message,
        status: 'pending' as const,
        createdAt: new Date().toISOString(),
        iterations: [],
        events: [],
      }
      
      // Pre-populate cache
      queryClient.setQueryData(
        queryKeys.task(optimisticTask.taskId),
        optimisticTask
      )
      
      return { optimisticTask }
    },
    
    onSuccess: async (data, variables, context) => {
      // Replace optimistic data with real data
      const fullTask = {
        ...data,
        iterations: [],
        events: [],
      }
      
      queryClient.setQueryData(
        queryKeys.task(data.taskId),
        fullTask
      )
      
      // Remove temp task if different ID
      if (context?.optimisticTask.taskId !== data.taskId) {
        queryClient.removeQueries({
          queryKey: queryKeys.task(context.optimisticTask.taskId)
        })
      }
      
      // Invalidate lists only (not all task details)
      await queryClient.invalidateQueries({ 
        queryKey: queryKeys.tasks.lists() 
      })
      
      // Navigate only after cache is ready
      navigate({ to: `/tasks/${data.taskId}` })
      
      options?.onSuccess?.(data, variables, context)
    },
    
    onError: (error, variables, context) => {
      // Rollback optimistic update
      if (context?.optimisticTask) {
        queryClient.removeQueries({
          queryKey: queryKeys.task(context.optimisticTask.taskId)
        })
      }
      options?.onError?.(error, variables, context)
    },
  })
}
```

2. **Remove navigation from index.tsx** (`client/src/routes/index.tsx`):
```typescript
// Change from:
const createTask = useCreateTask({
  onSuccess: (data) => {
    navigate({ to: `/tasks/${data.taskId}` })
  },
})

// To:
const createTask = useCreateTask()
// Navigation now handled inside the hook
```

### Phase 2: Fix SSE Connection Initialization (Priority: High)

#### Current Problems:
- SSE waits for metadata before connecting
- Early events can be missed
- Complex initialization logic

#### Refactoring Steps:

1. **Simplify `useTaskStreaming` hook** (`client/src/features/tasks/hooks/use-task-streaming.ts`):
```typescript
export function useTaskStreaming(taskId: string | undefined) {
  const queryClient = useQueryClient()
  const [isConnected, setIsConnected] = useState(false)
  const [reconnectAttempts, setReconnectAttempts] = useState(0)
  
  useEffect(() => {
    if (!taskId || taskId.startsWith('temp-')) return
    
    // Connect immediately with task ID
    const eventSource = new EventSource(`/api/tasks/${taskId}/events`)
    let lastEventId: string | undefined
    
    eventSource.onopen = () => {
      setIsConnected(true)
      setReconnectAttempts(0)
      console.log(`SSE connected for task ${taskId}`)
    }
    
    eventSource.onerror = () => {
      setIsConnected(false)
      console.error(`SSE error for task ${taskId}`)
      
      // Reconnect with exponential backoff
      const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000)
      setReconnectAttempts(prev => prev + 1)
      
      setTimeout(() => {
        eventSource.close()
        // React will re-run this effect
      }, delay)
    }
    
    eventSource.addEventListener('task-event', (event) => {
      const taskEvent = JSON.parse(event.data)
      lastEventId = taskEvent.id
      
      // Update events cache
      queryClient.setQueryData(
        queryKeys.tasks.events(taskId),
        (old: TaskEvent[] = []) => [...old, taskEvent]
      )
      
      // Update task status if changed
      if (taskEvent.type === 'status_change') {
        queryClient.setQueryData(
          queryKeys.task(taskId),
          (old) => old ? { ...old, status: taskEvent.status } : old
        )
      }
    })
    
    return () => {
      eventSource.close()
      setIsConnected(false)
    }
  }, [taskId])
  
  return { isConnected }
}
```

2. **Update `useTaskEventsWithStreaming`** to start SSE immediately:
```typescript
export function useTaskEventsWithStreaming(taskId: string) {
  // Start SSE connection immediately
  const { isConnected } = useTaskStreaming(taskId)
  
  // Fetch initial events (won't block SSE)
  const { data: events = [], isLoading } = useQuery({
    queryKey: queryKeys.tasks.events(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
    // Refetch periodically to catch any missed events
    refetchInterval: isConnected ? false : 5000,
  })
  
  return { events, isStreaming: isConnected, isLoading }
}
```

### Phase 3: Add Loading States to ChatView (Priority: High)

#### Current Problems:
- ChatView renders empty when data isn't ready
- No feedback during initial load

#### Refactoring Steps:

1. **Update ChatView** (`client/src/features/tasks/components/chat-view/chat-view.tsx`):
```typescript
export function ChatView() {
  const params = useParams({ from: '/tasks/$taskId' })
  const { data: task, isLoading: isLoadingTask } = useTask(params.taskId)
  const { events, isStreaming, isLoading: isLoadingEvents } = useTaskEventsWithStreaming(params.taskId)
  
  // Show skeleton during initial load
  if (isLoadingTask || !task) {
    return <ChatViewSkeleton />
  }
  
  // Show minimal UI while events are loading
  if (isLoadingEvents && events.length === 0) {
    return (
      <div className="flex flex-col h-full">
        <ChatMessages messages={[]} isLoading />
        <ChatInput disabled />
      </div>
    )
  }
  
  return (
    <div className="flex flex-col h-full">
      <ChatMessages messages={events} />
      <StreamingIndicator visible={isStreaming} />
      <ChatInput taskId={params.taskId} disabled={!task || task.status === 'completed'} />
    </div>
  )
}
```

### Phase 4: Implement Parallel Data Fetching (Priority: Medium)

#### Current Problems:
- Sequential data fetching causes waterfalls
- Slow initial page load

#### Refactoring Steps:

1. **Create parallel fetching hook** (`client/src/features/tasks/hooks/use-task-data.ts`):
```typescript
export function useTaskData(taskId: string) {
  const queries = useQueries({
    queries: [
      {
        queryKey: queryKeys.task(taskId),
        queryFn: () => tasksApi.getTask(taskId),
        staleTime: 5 * 60 * 1000,
      },
      {
        queryKey: queryKeys.tasks.events(taskId),
        queryFn: () => tasksApi.getTaskEvents(taskId),
        staleTime: 60 * 1000,
      },
      {
        queryKey: queryKeys.tasks.iterations(taskId),
        queryFn: () => tasksApi.getTaskIterations(taskId),
        staleTime: 2 * 60 * 1000,
      },
      {
        queryKey: queryKeys.tasks.gitDiff(taskId),
        queryFn: () => tasksApi.getGitDiff(taskId),
        staleTime: 10 * 60 * 1000,
        // Git diff is optional
        enabled: !!taskId && !taskId.startsWith('temp-'),
      },
    ],
  })
  
  return {
    task: queries[0].data,
    events: queries[1].data || [],
    iterations: queries[2].data || [],
    gitDiff: queries[3].data,
    isLoading: queries.some(q => q.isLoading && q.isFetching),
    error: queries.find(q => q.error)?.error,
  }
}
```

2. **Update TaskProvider to use parallel fetching**:
```typescript
export function TaskProvider({ taskId, children }: TaskProviderProps) {
  const { task, events, iterations, gitDiff, isLoading, error } = useTaskData(taskId)
  const { isConnected } = useTaskStreaming(taskId)
  
  const value = {
    task,
    events,
    iterations,
    gitDiff,
    isLoading,
    error,
    isStreaming: isConnected,
  }
  
  return (
    <TaskContext.Provider value={value}>
      {children}
    </TaskContext.Provider>
  )
}
```

### Expected Outcomes

#### After Phase 1:
- ✅ Instant navigation after task creation
- ✅ No empty ChatView on initial render  
- ✅ Smooth transition from optimistic to real data

#### After Phase 2:
- ✅ SSE connects immediately with task ID
- ✅ No missed early events
- ✅ Automatic reconnection on failures

#### After Phase 3:
- ✅ Proper loading states during data fetch
- ✅ Better perceived performance
- ✅ Clear user feedback

#### After Phase 4:
- ✅ Faster initial page load
- ✅ All data fetched in parallel
- ✅ Reduced time to interactive

### Testing Strategy

1. **Test Optimistic Updates**:
   - Create task and verify immediate cache update
   - Test rollback on failure
   - Verify navigation timing

2. **Test SSE Lifecycle**:
   - Connection starts with just task ID
   - Events update cache correctly
   - Reconnection works on failure

3. **Test Loading States**:
   - ChatView shows skeleton initially
   - Transitions smoothly when data arrives
   - Handles errors gracefully

## Best Practices

### 1. React Query Best Practices
- Use optimistic updates for mutations
- Set appropriate staleTime to reduce refetches
- Use prefetchQuery for predictable navigation
- Leverage query invalidation for cache updates

### 2. SSE Connection Management
- Start connection as soon as task ID is available
- Implement automatic reconnection with backoff
- Sync with API data to avoid gaps
- Clean up connections on unmount

### 3. Component Design
- Keep components focused on single responsibility
- Use custom hooks for data fetching logic
- Handle loading and error states consistently
- Implement proper TypeScript types

### 4. User Experience
- Show immediate feedback with optimistic updates
- Provide clear error messages with retry options
- Use skeleton loaders instead of spinners
- Maintain UI state during data refetches

## Conclusion

This simplified architecture leverages React Query's powerful features to create a clean, maintainable task management system. By avoiding custom abstractions and focusing on React Query's built-in capabilities, we achieve a robust solution without unnecessary complexity. The key is to trust React Query for cache management while keeping our business logic simple and focused.
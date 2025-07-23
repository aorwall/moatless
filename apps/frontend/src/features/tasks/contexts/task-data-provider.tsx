import type { Event, GitDiffResponse, TaskIteration, TaskResponse } from '@moatless/types'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react'
import { useGlobalEventStream, useTaskEventSubscription } from '@/features/events'
import { queryKeys } from '@/lib/api/query-keys'
import { tasksApi } from '../api/tasks'

interface TaskDataContextValue {
  // Task metadata (rarely changes)
  taskId: string
  status: string | undefined
  agentType: string | undefined
  repositoryId: string | undefined
  iterations: TaskIteration[]
  createdAt: Date | undefined

  // Git diff data
  gitDiff: GitDiffResponse | undefined

  // Derived flags for components (stable primitives)
  canExecute: boolean
  isRunning: boolean
  isComplete: boolean
  runningStartTime: Date | undefined

  // Event data
  events: Event[] // Historical events from API
  streamingEvents: Event[] // Live events from SSE (for minified viewer)
  isStreaming: boolean
  streamingError: string | null
  latestEventId: string | null
  hasNewEvents: boolean

  // Loading & error states
  isLoading: boolean
  error: Error | null
}

const TaskDataContext = createContext<TaskDataContextValue | undefined>(undefined)

interface TaskDataProviderProps {
  taskId: string
  children: ReactNode
}

/**
 * TaskDataProvider manages stable task metadata, workspace status, and events.
 * Handles both historical events (from API) and live streaming events (from SSE).
 * Uses React Query without complex selectors to minimize re-renders.
 */
export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  const queryClient = useQueryClient()
  const globalStream = useGlobalEventStream()

  // Local state for streaming events (recent events from SSE)
  const [streamingEvents, setStreamingEvents] = useState<Event[]>([])
  // Task metadata query - simplified without select
  const taskQuery = useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    staleTime: 30000, // 30s - task metadata doesn't change frequently
    enabled: !!taskId && !taskId.startsWith('temp-'),
  })

  // Git diff query
  const gitDiffQuery = useQuery({
    queryKey: queryKeys.tasks.gitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-'),
    staleTime: 10 * 60 * 1000, // 10 minutes - git diff doesn't change frequently
    retry: false, // Don't retry on failure
  })

  // Events query - historical events from API
  const eventsQuery = useQuery({
    queryKey: queryKeys.tasks.events(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
    staleTime: Number.POSITIVE_INFINITY, // Never stale - updated via SSE only
    enabled: !!taskId && !taskId.startsWith('temp-'),
    // No polling - events are updated via SSE
  })

  // Event handler - updates both historical and streaming events
  const handleTaskEvent = useCallback(
    (event: Event) => {
      // Update historical events cache
      queryClient.setQueryData(queryKeys.tasks.events(taskId), (old: Event[] = []) => {
        const eventMap = new Map(old.map((e) => [e.eventId, e]))
        eventMap.set(event.eventId, event)
        return Array.from(eventMap.values()).sort((a, b) => {
          const aTime = a.timestamp ? new Date(a.timestamp).getTime() : 0
          const bTime = b.timestamp ? new Date(b.timestamp).getTime() : 0
          return aTime - bTime
        })
      })

      // Update streaming events (keep last 20 events for the minified viewer)
      setStreamingEvents((prev) => {
        const updated = [...prev, event]
        return updated.slice(-20) // Keep only the last 20 events
      })
    },
    [taskId, queryClient],
  )

  // Subscribe to task-specific events from global stream
  useEffect(() => {
    console.log('[TaskDataProvider] Setting up event subscription for task:', taskId)
  }, [taskId])

  useTaskEventSubscription(taskId, handleTaskEvent)

  // Extract primitive values in useMemo to ensure stability
  const taskPrimitives = useMemo(() => {
    const task: TaskResponse | undefined = taskQuery.data
    if (!task) return null

    const iterations = task.iterations || []
    const lastIteration = iterations.length > 0 ? iterations[iterations.length - 1] : null

    // Derive running state from last iteration status for now
    // TODO: This will need to come from workspace status when components are updated
    const isRunning = lastIteration?.status === 'active'
    const runningStartTime =
      isRunning && lastIteration?.startTime ? new Date(lastIteration.startTime) : undefined

    return {
      status: task.status,
      agentType: task.agentType,
      repositoryId: task.repositoryId,
      iterations,
      createdAt: task.createdAt,
      isRunning,
      canExecute: task.status === 'open', // Can execute if task is open
      isComplete: task.status === 'closed', // Complete if task is closed
      runningStartTime,
    }
  }, [taskQuery.data])

  // Event primitives
  const eventPrimitives = useMemo(() => {
    const events = eventsQuery.data || []
    const latestEventId = events.length > 0 ? events[events.length - 1].eventId : null

    return {
      events,
      latestEventId,
      hasNewEvents: false, // Not needed with single source of truth
    }
  }, [eventsQuery.data])

  // Streaming status from global stream
  const isStreaming = globalStream.isConnected
  const streamingError = globalStream.connectionError?.message || null

  // Memoized context value with stable primitive dependencies
  const contextValue: TaskDataContextValue = useMemo(
    () => ({
      // Task metadata primitives
      taskId,
      status: taskPrimitives?.status,
      agentType: taskPrimitives?.agentType,
      repositoryId: taskPrimitives?.repositoryId,
      iterations: taskPrimitives?.iterations || [],
      createdAt: taskPrimitives?.createdAt,

      // Git diff data
      gitDiff: gitDiffQuery.data,

      // Derived flags (stable primitives)
      canExecute: taskPrimitives?.canExecute ?? false,
      isRunning: taskPrimitives?.isRunning ?? false,
      isComplete: taskPrimitives?.isComplete ?? false,
      runningStartTime: taskPrimitives?.runningStartTime,

      // Event data
      events: eventPrimitives.events,
      streamingEvents, // Live events for minified viewer
      isStreaming,
      streamingError,
      latestEventId: eventPrimitives.latestEventId,
      hasNewEvents: eventPrimitives.hasNewEvents,

      // Loading & error states - only block on core task data
      isLoading: taskQuery.isLoading,
      error: taskQuery.error || null,
    }),
    [
      taskId,
      taskPrimitives,
      gitDiffQuery.data,
      eventPrimitives,
      streamingEvents,
      isStreaming,
      streamingError,
      taskQuery.isLoading,
      taskQuery.error,
    ],
  )

  return <TaskDataContext.Provider value={contextValue}>{children}</TaskDataContext.Provider>
}

/**
 * Hook to access stable task data and status.
 * Returns primitives that only change when actual values change.
 */
export function useTaskData() {
  const context = useContext(TaskDataContext)
  if (!context) {
    throw new Error('useTaskData must be used within TaskDataProvider')
  }
  return context
}

// Specialized hooks for specific data slices
export function useTaskStatus() {
  const { status, isRunning, canExecute, isComplete } = useTaskData()
  return { status, isRunning, canExecute, isComplete }
}

export function useTaskIterations() {
  const { iterations } = useTaskData()
  return iterations
}

export function useGitDiff() {
  const { gitDiff } = useTaskData()
  return gitDiff
}

export function useGitDiffWithLoading() {
  const { taskId } = useTaskData()
  const gitDiffQuery = useQuery({
    queryKey: queryKeys.tasks.gitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-'),
    staleTime: 10 * 60 * 1000, // 10 minutes - git diff doesn't change frequently
    retry: false, // Don't retry on failure
  })

  return {
    gitDiff: gitDiffQuery.data,
    isLoading: gitDiffQuery.isLoading,
    error: gitDiffQuery.error,
  }
}

export function useTaskDuration() {
  const { isRunning, runningStartTime } = useTaskData()
  return { isRunning, runningStartTime }
}

// Performance optimized hooks that only return specific fields
export function useTaskLoadingError() {
  const { isLoading, error } = useTaskData()
  return { isLoading, error }
}

export function useTaskMetadata() {
  const { taskId, agentType, repositoryId, createdAt } = useTaskData()
  return { taskId, agentType, repositoryId, createdAt }
}

// Event hooks (previously in TaskEventsProvider)
export function useTaskEvents() {
  const { events, streamingEvents, isStreaming, streamingError, latestEventId, hasNewEvents } =
    useTaskData()
  return {
    events,
    streamingEvents,
    isStreaming,
    streamingError,
    latestEventId,
    hasNewEvents,
    error: streamingError ? new Error(streamingError) : null,
    isLoading: false, // Events are always available
  }
}

export function useEventStream() {
  const { streamingEvents, isStreaming, latestEventId } = useTaskData()
  // For compatibility - return streaming events for components like MinifiedLastEventsDisplay
  return { events: streamingEvents, isStreaming, latestEventId }
}

export function useStreamingStatus() {
  const { isStreaming, streamingError, hasNewEvents } = useTaskData()
  return { isStreaming, streamingError, hasNewEvents }
}

/**
 * Hook to get the latest context tokens from message events.
 * Returns the contextTokens from the most recent message event.
 */
export function useLatestContextTokens() {
  const { events } = useTaskData()

  return useMemo(() => {
    // Find the last message event with contextTokens
    for (let i = events.length - 1; i >= 0; i--) {
      const event = events[i]
      if (event.type === 'agent' && event.subType === 'message' && event.contextTokens) {
        return event.contextTokens
      }
    }
    return 0
  }, [events])
}

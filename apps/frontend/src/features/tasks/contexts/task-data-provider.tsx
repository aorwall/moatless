import type {
  Event,
  GitDiffResponse,
  TaskIteration,
  TaskResponse,
  WorkspaceStatusResponse,
} from '@moatless/types'
import { useQuery } from '@tanstack/react-query'
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
  vibe: boolean | undefined
  templateId: string | undefined
  name: string | undefined
  description: string | undefined

  // Git diff data
  gitDiff: GitDiffResponse | undefined

  // Workspace status
  workspaceStatus: WorkspaceStatusResponse | undefined

  // Derived flags for components (stable primitives)
  canExecute: boolean
  isAgentRunning: boolean
  isComplete: boolean

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

  // Workspace status query
  const workspaceQuery = useQuery({
    queryKey: queryKeys.tasks.workspaceStatus(taskId),
    queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-') && taskQuery.data?.status === 'open',
    refetchInterval: (query) => {
      const data = query.state.data as WorkspaceStatusResponse | undefined
      const envStatus = data?.environmentStatus
      const agentState = data?.agentState

      // Only poll when in transitional states
      if (envStatus === 'initializing' || envStatus === 'cleaning' || agentState === 'running') {
        return 3000 // 3 seconds for transitional states
      }
      return false // Stop polling when stable
    },
    staleTime: 2000, // Consider data stale after 2 seconds
  })

  // Events query - historical events from API
  const eventsQuery = useQuery({
    queryKey: queryKeys.tasks.events(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
    staleTime: Number.POSITIVE_INFINITY, // Never stale - updated via SSE only
    enabled: !!taskId && !taskId.startsWith('temp-'),
    // No polling - events are updated via SSE
  })

  // Event handler - updates only local streaming events for display
  const handleTaskEvent = useCallback((event: Event) => {
    // Update streaming events (keep last 20 events for the minified viewer)
    setStreamingEvents((prev) => {
      const updated = [...prev, event]
      return updated.slice(-20) // Keep only the last 20 events
    })
  }, [])

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

    return {
      status: task.status,
      agentType: task.agentType,
      repositoryId: task.repositoryId,
      iterations,
      createdAt: task.createdAt,
      vibe: task.vibe,
      templateId: task.templateId,
      name: task.name,
      description: task.description,
      canExecute: task.status === 'open', // Can execute if task is open
      isComplete: task.status === 'closed', // Complete if task is closed
    }
  }, [taskQuery.data])

  // Derive agent running state from task status
  const isAgentRunning = useMemo(
    () => taskPrimitives?.status === 'running',
    [taskPrimitives?.status],
  )

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
      vibe: taskPrimitives?.vibe,
      templateId: taskPrimitives?.templateId,
      name: taskPrimitives?.name,
      description: taskPrimitives?.description,

      // Git diff data
      gitDiff: gitDiffQuery.data,

      // Workspace status
      workspaceStatus: workspaceQuery.data,

      // Derived flags (stable primitives)
      canExecute: taskPrimitives?.canExecute ?? false,
      isAgentRunning,
      isComplete: taskPrimitives?.isComplete ?? false,

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
      workspaceQuery.data,
      isAgentRunning,
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
  const { status, isAgentRunning, canExecute, isComplete } = useTaskData()
  return { status, isAgentRunning, canExecute, isComplete }
}

export function useIsAgentRunning() {
  const { isAgentRunning } = useTaskData()
  return isAgentRunning
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

export function useWorkspaceStatus() {
  const { workspaceStatus } = useTaskData()
  return workspaceStatus
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

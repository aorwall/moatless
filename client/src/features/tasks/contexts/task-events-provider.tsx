import { queryKeys } from '@/lib/api/query-keys'
import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
    createContext,
    type ReactNode,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
} from 'react'
import { NDJSONSSEClient } from '../api/ndjson-sse'
import { tasksApi } from '../api/tasks'

// Define which event types trigger task data updates
const STATUS_AFFECTING_EVENTS = [
  EventType.RESULT, // Task completed successfully
  EventType.ERROR, // Task failed
  EventType.STOP, // Agent stopped
  EventType.START, // Agent started
] as const

interface TaskEventsContextValue {
  events: Event[]
  isStreaming: boolean
  streamingError: string | null

  // Event-specific data with stable references
  latestEventId: string | null
  hasNewEvents: boolean

  // Loading states
  isLoading: boolean
  error: Error | null
}

const TaskEventsContext = createContext<TaskEventsContextValue | undefined>(undefined)

interface TaskEventsProviderProps {
  taskId: string
  children: ReactNode
}

/**
 * TaskEventsProvider manages real-time events and event-driven updates.
 * Implements smart event merging and status-affecting event handling.
 */
export function TaskEventsProvider({ taskId, children }: TaskEventsProviderProps) {
  const queryClient = useQueryClient()
  const sseClientRef = useRef<NDJSONSSEClient | null>(null)
  const [isStreaming, setIsStreaming] = useState(false)
  const [streamingError, setStreamingError] = useState<string | null>(null)

  // Get events from React Query - this is the ONLY source of truth
  const eventsQuery = useQuery({
    queryKey: queryKeys.taskEvents(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
    staleTime: 60 * 1000, // 1 minute
    enabled: !!taskId && !taskId.startsWith('temp-'),
    refetchInterval: isStreaming ? false : 5000, // Poll only when not streaming
  })

  // Simplified event handler that only updates React Query cache
  const handleTaskEvent = useCallback(
    (event: Event) => {
      // Update React Query cache directly - single source of truth
      queryClient.setQueryData(queryKeys.taskEvents(taskId), (old: Event[] = []) => {
        const eventMap = new Map(old.map((e) => [e.eventId, e]))
        eventMap.set(event.eventId, event)
        return Array.from(eventMap.values()).sort((a, b) => {
          const aTime = a.timestamp ? new Date(a.timestamp).getTime() : 0
          const bTime = b.timestamp ? new Date(b.timestamp).getTime() : 0
          return aTime - bTime
        })
      })

      // Refetch task data for status-affecting events
      if ((STATUS_AFFECTING_EVENTS as readonly EventType[]).includes(event.type)) {
        console.log(`[Events] Status-affecting event ${event.type}, refetching task data`)
        
        // Use queueMicrotask to ensure cache update completes before invalidation
        queueMicrotask(() => {
          queryClient.invalidateQueries({
            queryKey: queryKeys.task(taskId),
            refetchType: 'active',
          })
          
          queryClient.invalidateQueries({
            queryKey: queryKeys.taskWorkspaceStatus(taskId),
            refetchType: 'active',
          })
        })
      }
    },
    [taskId, queryClient],
  )

  // SSE connection management
  const shouldStream = useMemo(() => {
    return !!taskId && !taskId.startsWith('temp-')
  }, [taskId])

  const handleSSEEvent = useCallback(
    (event: Event) => {
      handleTaskEvent(event)
      setStreamingError(null)
    },
    [handleTaskEvent],
  )

  const handleSSEError = useCallback((error: Error) => {
    console.error('SSE error:', error)
    if (error.message.includes('Task not found')) {
      setStreamingError('Task not found')
    } else {
      setStreamingError('Streaming connection error')
    }
  }, [])

  const handleSSEConnect = useCallback(() => {
    console.log(`[SSE] Connected for task ${taskId}`)
    setIsStreaming(true)
    setStreamingError(null)
  }, [taskId])

  const handleSSEDisconnect = useCallback(() => {
    console.log(`[SSE] Disconnected for task ${taskId}`)
    setIsStreaming(false)
  }, [taskId])

  // SSE connection effect
  useEffect(() => {
    if (!shouldStream) {
      return
    }

    console.log(`[SSE] Starting streaming for task ${taskId}`)

    const client = new NDJSONSSEClient({
      taskId,
      onEvent: handleSSEEvent,
      onError: handleSSEError,
      onConnect: handleSSEConnect,
      onDisconnect: handleSSEDisconnect,
      onTaskNotFound: () => setStreamingError('Task not found'),
    })

    sseClientRef.current = client
    client.connect()

    return () => {
      if (sseClientRef.current) {
        sseClientRef.current.disconnect()
        sseClientRef.current = null
      }
      setIsStreaming(false)
      setStreamingError(null)
    }
  }, [shouldStream, taskId, handleSSEEvent, handleSSEError, handleSSEConnect, handleSSEDisconnect])

  // Get latest event ID for stable reference
  const latestEventId = useMemo(() => {
    const events = eventsQuery.data || []
    return events.length > 0 ? events[events.length - 1].eventId : null
  }, [eventsQuery.data])

  // Simplified context value - React Query is the single source of truth
  const contextValue: TaskEventsContextValue = useMemo(
    () => ({
      events: eventsQuery.data || [],
      isStreaming,
      streamingError,
      latestEventId,
      hasNewEvents: false, // Not needed with single source of truth
      isLoading: eventsQuery.isLoading,
      error: streamingError ? new Error(streamingError) : eventsQuery.error || null,
    }),
    [
      eventsQuery.data,
      eventsQuery.isLoading,
      eventsQuery.error,
      isStreaming,
      streamingError,
      latestEventId,
    ],
  )

  return <TaskEventsContext.Provider value={contextValue}>{children}</TaskEventsContext.Provider>
}

/**
 * Hook to access event stream data.
 * Returns stable references that only change when events actually update.
 */
export function useTaskEvents() {
  const context = useContext(TaskEventsContext)
  if (!context) {
    throw new Error('useTaskEvents must be used within TaskEventsProvider')
  }
  return context
}

// Specialized hooks for specific event data
export function useEventStream() {
  const { events, isStreaming, latestEventId } = useTaskEvents()
  return { events, isStreaming, latestEventId }
}

export function useStreamingStatus() {
  const { isStreaming, streamingError, hasNewEvents } = useTaskEvents()
  return { isStreaming, streamingError, hasNewEvents }
}

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
import type { Event } from '@/lib/types/events'

interface TaskDataContextValue {
  taskId: string
  streamingEvents: Event[]
  isStreaming: boolean
  streamingError: string | null
}

const TaskDataContext = createContext<TaskDataContextValue | undefined>(undefined)

interface TaskDataProviderProps {
  taskId: string
  children: ReactNode
}

export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  const globalStream = useGlobalEventStream()

  // Local state for streaming events (recent events from SSE)
  const [streamingEvents, setStreamingEvents] = useState<Event[]>([])

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
    if (!taskId) {
      return
    }
    // Reset stream buffer whenever the task changes so consumers get a clean slate
    setStreamingEvents([])
  }, [taskId])

  useTaskEventSubscription(taskId, handleTaskEvent)

  // Streaming status from global stream
  const isStreaming = globalStream.isConnected
  const streamingError = globalStream.connectionError?.message || null

  // Memoized context value with stable primitive dependencies
  const contextValue: TaskDataContextValue = useMemo(
    () => ({
      taskId,
      streamingEvents,
      isStreaming,
      streamingError,
    }),
    [taskId, streamingEvents, isStreaming, streamingError],
  )

  return <TaskDataContext.Provider value={contextValue}>{children}</TaskDataContext.Provider>
}

// Event hooks (previously in TaskEventsProvider)
export function useTaskEvents() {
  const { streamingEvents, isStreaming, streamingError } = useTaskDataContext()
  return {
    events: streamingEvents, // Only streaming events now
    streamingEvents,
    isStreaming,
    streamingError,
    error: streamingError ? new Error(streamingError) : null,
    isLoading: false, // Events are always available
  }
}

export function useEventStream() {
  const { streamingEvents, isStreaming } = useTaskDataContext()
  // For compatibility - return streaming events for components like MinifiedLastEventsDisplay
  return { events: streamingEvents, isStreaming }
}

export function useStreamingStatus() {
  const { isStreaming, streamingError } = useTaskDataContext()
  return { isStreaming, streamingError }
}

/**
 * Hook to get the latest context tokens from message events.
 * Returns the contextTokens from the most recent message event.
 */
export function useLatestContextTokens() {
  const { streamingEvents } = useTaskDataContext()

  return useMemo(() => {
    // Find the last message event with contextTokens
    for (let i = streamingEvents.length - 1; i >= 0; i--) {
      const event = streamingEvents[i]
      if (
        event.type === 'agent' &&
        event.subType === 'message' &&
        typeof event.payload?.contextTokens === 'number'
      ) {
        return event.payload.contextTokens
      }
    }
    return 0
  }, [streamingEvents])
}

function useTaskDataContext() {
  const context = useContext(TaskDataContext)
  if (!context) {
    throw new Error('Task data hooks must be used within TaskDataProvider')
  }
  return context
}

export function useTaskId() {
  const { taskId } = useTaskDataContext()
  return taskId
}

export function useTaskStreamingState() {
  const { streamingEvents, isStreaming, streamingError } = useTaskDataContext()
  return { streamingEvents, isStreaming, streamingError }
}

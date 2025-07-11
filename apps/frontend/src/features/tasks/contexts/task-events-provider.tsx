import type { Event } from '@moatless/shared/types/event'
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
import {
  getInvalidationKeys,
  shouldInvalidateTaskList,
  useGlobalEventStream,
  useTaskEventSubscription,
} from '@/features/events'
import { queryKeys } from '@/lib/api/query-keys'
import { tasksApi } from '../api/tasks'

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
 * Uses the global event stream for all task events.
 */
export function TaskEventsProvider({ taskId, children }: TaskEventsProviderProps) {
  const queryClient = useQueryClient()

  // Get global stream connection status
  const globalStream = useGlobalEventStream()

  // Get events from React Query - this is the ONLY source of truth
  const eventsQuery = useQuery({
    queryKey: queryKeys.taskEvents(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId),
    staleTime: 60 * 1000, // 1 minute
    enabled: !!taskId && !taskId.startsWith('temp-'),
    // No polling - events are updated via SSE only
  })

  // Smart event handler with invalidation mapping
  const handleTaskEvent = useCallback(
    (event: Event) => {
      // 1. Update React Query cache directly - single source of truth
      queryClient.setQueryData(queryKeys.taskEvents(taskId), (old: Event[] = []) => {
        const eventMap = new Map(old.map((e) => [e.eventId, e]))
        eventMap.set(event.eventId, event)
        return Array.from(eventMap.values()).sort((a, b) => {
          const aTime = a.timestamp ? new Date(a.timestamp).getTime() : 0
          const bTime = b.timestamp ? new Date(b.timestamp).getTime() : 0
          return aTime - bTime
        })
      })

      // 2. Get invalidation keys based on event type
      const invalidationKeys = getInvalidationKeys(event)

      if (invalidationKeys.length > 0) {
        console.log(
          `[Events] Event ${event.type} triggering invalidations:`,
          invalidationKeys.map((fn) => fn.name),
        )

        // Use queueMicrotask to ensure cache update completes before invalidation
        queueMicrotask(() => {
          // 3. Invalidate each affected query
          invalidationKeys.forEach((keyFunction) => {
            queryClient.invalidateQueries({
              queryKey: keyFunction(taskId),
              refetchType: 'active',
            })
          })

          // 4. Always invalidate task lists for status-affecting events
          if (shouldInvalidateTaskList(event)) {
            queryClient.invalidateQueries({
              queryKey: queryKeys.tasksList(),
              refetchType: 'active',
            })
          }
        })
      }
    },
    [taskId, queryClient],
  )

  // Subscribe to task-specific events from global stream
  useEffect(() => {
    console.log('[TaskEventsProvider] Setting up subscription for task:', taskId)
  }, [taskId])

  useTaskEventSubscription(taskId, handleTaskEvent)

  // Derive streaming status directly from global stream
  const isStreaming = globalStream.isConnected
  const streamingError = globalStream.connectionError?.message || null

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

/**
 * Hook to get the latest context tokens from message events.
 * Returns the contextTokens from the most recent message event.
 */
export function useLatestContextTokens() {
  const { events } = useTaskEvents()

  return useMemo(() => {
    // Find the last message event with contextTokens
    for (let i = events.length - 1; i >= 0; i--) {
      const event = events[i]
      if (event.type === 'message' && event.contextTokens) {
        return event.contextTokens
      }
    }
    return 0
  }, [events])
}

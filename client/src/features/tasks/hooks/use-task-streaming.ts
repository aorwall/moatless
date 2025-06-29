import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { NDJSONSSEClient } from '@/features/tasks/api/ndjson-sse'
import { useTask } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UseTaskStreamingResult {
  streamEvents: Array<Event>
  isStreaming: boolean
  streamingError: string | null
}

/**
 * Hook that manages streaming events using local React state instead of Zustand.
 * This avoids the infinite render issue while providing streaming functionality.
 */
export function useTaskStreaming(taskId: string): UseTaskStreamingResult {
  const sseClientRef = useRef<NDJSONSSEClient | null>(null)
  const [streamEvents, setStreamEvents] = useState<Array<Event>>([])
  const [isStreaming, setIsStreaming] = useState(false)
  const [streamingError, setStreamingError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Use refs to avoid effect re-runs
  const reconnectAttemptsRef = useRef(0)
  const reconnectTimerRef = useRef<NodeJS.Timeout | undefined>()

  // Get task metadata (only for status updates, not for connection decision)
  const { data: metadata } = useTask(taskId)

  // Determine if we should stream - only for active tasks
  const shouldStream = useMemo(() => {
    // Don't stream for temporary/optimistic task IDs
    if (!taskId || taskId.startsWith('temp-')) return false

    // Only stream for tasks that are actively generating events
    // - 'running': Actively executing and generating events
    // - 'open': Waiting for execution, no events yet
    // - 'error', 'merged', 'closed': Complete/failed, no more events expected
    if (metadata?.status) {
      return metadata.status === 'running'
    }

    // If we don't have metadata yet, optimistically stream
    // This handles the case where task is just created and metadata is loading
    return true
  }, [taskId, metadata?.status])

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEEvent = useCallback(
    (newEvent: Event) => {
      setStreamEvents((prev) => {
        // Check if event already exists to prevent duplicates
        if (prev.some((e) => e.eventId === newEvent.eventId)) {
          return prev
        }
        return [...prev, newEvent]
      })
      setStreamingError(null)

      // Update cache based on event type
      if (newEvent.type === EventType.RESULT) {
        // Refetch task metadata to update status
        queryClient.invalidateQueries({ queryKey: queryKeys.task(taskId) })
      }

      // Also update events cache directly
      queryClient.setQueryData(queryKeys.taskEvents(taskId), (old: Event[] = []) => [
        ...old,
        newEvent,
      ])
    },
    [queryClient, taskId],
  )

  const handleSSEError = useCallback((error: Error) => {
    console.error('SSE error:', error)
    if (error.message.includes('Task not found')) {
      setStreamingError('Task not found')
    } else if (error.message.includes('connection failed')) {
      setStreamingError('Connection failed after multiple attempts')
    } else {
      setStreamingError('Streaming connection error')
    }
  }, [])

  const handleSSEConnect = useCallback(() => {
    console.log(`[SSE] Connected for task ${taskId}`)
    setIsStreaming(true)
    setStreamingError(null)
    reconnectAttemptsRef.current = 0 // Reset reconnect attempts on successful connection
  }, [taskId])

  const handleSSEDisconnect = useCallback(() => {
    console.log(`[SSE] Disconnected for task ${taskId}`)
    setIsStreaming(false)
  }, [taskId])

  const handleSSETaskNotFound = useCallback(() => {
    setStreamingError('Task not found')
    setIsStreaming(false)
  }, [])

  // Create a stable disconnect handler that manages reconnection
  const handleSSEDisconnectWithRetry = useCallback(() => {
    handleSSEDisconnect()

    // Implement exponential backoff for reconnection
    if (reconnectAttemptsRef.current < 5 && shouldStream) {
      const delay = Math.min(1000 * 2 ** reconnectAttemptsRef.current, 30000)
      console.log(
        `[SSE] Will reconnect in ${delay}ms (attempt ${reconnectAttemptsRef.current + 1})`,
      )
      reconnectAttemptsRef.current += 1

      // Clear any existing timer
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current)
      }

      // Set up reconnection
      reconnectTimerRef.current = setTimeout(() => {
        if (sseClientRef.current) {
          console.log(`[SSE] Attempting reconnection for task ${taskId}`)
          sseClientRef.current.connect()
        }
      }, delay)
    }
  }, [handleSSEDisconnect, shouldStream, taskId])

  // Manage SSE connection lifecycle
  // biome-ignore lint/correctness/useExhaustiveDependencies: Intentionally omitting callback dependencies to prevent reconnection loops
  useEffect(() => {
    // Prevent multiple connections
    if (sseClientRef.current) {
      console.log(`[SSE] Connection already exists for task ${taskId}, skipping`)
      return
    }

    if (shouldStream) {
      console.log(`[SSE] Starting streaming for task ${taskId}`)

      // Create and connect SSE client
      const client = new NDJSONSSEClient({
        taskId,
        onEvent: handleSSEEvent,
        onError: handleSSEError,
        onConnect: handleSSEConnect,
        onDisconnect: handleSSEDisconnectWithRetry,
        onTaskNotFound: handleSSETaskNotFound,
      })

      sseClientRef.current = client
      client.connect()

      return () => {
        console.log(`[SSE] Cleaning up streaming for task ${taskId}`)
        if (reconnectTimerRef.current) {
          clearTimeout(reconnectTimerRef.current)
          reconnectTimerRef.current = undefined
        }
        if (sseClientRef.current) {
          sseClientRef.current.disconnect()
          sseClientRef.current = null
        }
      }
    } else {
      // Clean up when not streaming
      if (sseClientRef.current) {
        console.log(`[SSE] Stopping streaming for task ${taskId} - shouldStream is false`)
        sseClientRef.current.disconnect()
        sseClientRef.current = null
      }
      setIsStreaming(false)
    }
  }, [shouldStream, taskId])

  // Clean up when component unmounts
  useEffect(() => {
    return () => {
      console.log('[SSE] Component unmounting, cleaning up')
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current)
        reconnectTimerRef.current = undefined
      }
      if (sseClientRef.current) {
        sseClientRef.current.disconnect()
        sseClientRef.current = null
      }
    }
  }, [])

  return {
    streamEvents,
    isStreaming,
    streamingError,
  }
}

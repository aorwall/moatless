import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import type { Task } from '@shared/types/task'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { NDJSONSSEClient } from '@/features/tasks/api/ndjson-sse'
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
export function useTaskStreaming(taskId: string, task?: Task): UseTaskStreamingResult {
  const sseClientRef = useRef<NDJSONSSEClient | null>(null)
  const [streamEvents, setStreamEvents] = useState<Array<Event>>([])
  const [isStreaming, setIsStreaming] = useState(false)
  const [streamingError, setStreamingError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Use refs to avoid effect re-runs
  const reconnectAttemptsRef = useRef(0)
  const reconnectTimerRef = useRef<NodeJS.Timeout | undefined>(undefined)

  // Determine if we should stream - only for active tasks
  const shouldStream = useMemo(() => {
    // Don't stream for temporary/optimistic task IDs
    if (!taskId || taskId.startsWith('temp-')) return false

    // Only stream for tasks that are actively generating events
    // - 'running': Actively executing and generating events
    // - 'open': Waiting for execution, no events yet
    // - 'error', 'merged', 'closed': Complete/failed, no more events expected
    if (task?.status) {
      return task.status === 'running'
    }

    // If we don't have task data yet, optimistically stream
    // This handles the case where task is just created and metadata is loading
    return true
  }, [taskId, task?.status])

  // Cleanup function
  const cleanup = useCallback(() => {
    if (sseClientRef.current) {
      sseClientRef.current.disconnect()
      sseClientRef.current = null
    }
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current)
      reconnectTimerRef.current = undefined
    }
    setIsStreaming(false)
    setStreamingError(null)
    reconnectAttemptsRef.current = 0
  }, [])

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
      if (newEvent.type === EventType.RESULT || newEvent.type === EventType.TOOL) {
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

  // Connect to SSE stream
  const connect = useCallback(() => {
    if (!shouldStream || sseClientRef.current) {
      return
    }

    console.log(`[SSE] Starting streaming for task ${taskId}`)

    // Create and connect SSE client
    const client = new NDJSONSSEClient({
      taskId,
      onEvent: handleSSEEvent,
      onError: handleSSEError,
      onConnect: handleSSEConnect,
      onDisconnect: handleSSEDisconnect,
      onTaskNotFound: handleSSETaskNotFound,
    })

    sseClientRef.current = client
    client.connect()
  }, [
    shouldStream,
    taskId,
    handleSSEEvent,
    handleSSEError,
    handleSSEConnect,
    handleSSEDisconnect,
    handleSSETaskNotFound,
  ])

  // Effect to manage connection lifecycle
  useEffect(() => {
    if (shouldStream) {
      connect()
    } else {
      cleanup()
    }

    return cleanup
  }, [shouldStream, connect, cleanup])

  // Clear events when task changes
  useEffect(() => {
    setStreamEvents([])
    setStreamingError(null)
    reconnectAttemptsRef.current = 0
  }, [])

  return {
    streamEvents,
    isStreaming,
    streamingError,
  }
}

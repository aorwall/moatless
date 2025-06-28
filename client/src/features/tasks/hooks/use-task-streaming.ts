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

  // Get task metadata (only for status updates, not for connection decision)
  const { data: metadata } = useTask(taskId)

  // Track reconnect attempts
  const [reconnectAttempts, setReconnectAttempts] = useState(0)

  // Determine if we should stream - connect immediately with valid task ID
  const shouldStream = useMemo(() => {
    // Don't stream for temporary/optimistic task IDs
    if (!taskId || taskId.startsWith('temp-')) return false

    // Stream immediately for any valid task ID
    // This ensures we don't miss early events while waiting for metadata
    return true
  }, [taskId])

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEEvent = useCallback(
    (newEvent: Event) => {
      setStreamEvents((prev) => [...prev, newEvent])
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

      // Update task status if we get a status change event
      if (newEvent.type === EventType.STATUS && metadata) {
        queryClient.setQueryData(queryKeys.task(taskId), {
          ...metadata,
          status: newEvent.payload?.status || metadata.status,
        })
      }
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
    setReconnectAttempts(0) // Reset reconnect attempts on successful connection
  }, [taskId])

  const handleSSEDisconnect = useCallback(() => {
    console.log(`[SSE] Disconnected for task ${taskId}`)
    setIsStreaming(false)

    // Implement exponential backoff for reconnection
    if (reconnectAttempts < 5) {
      const delay = Math.min(1000 * 2 ** reconnectAttempts, 30000)
      console.log(`[SSE] Will reconnect in ${delay}ms (attempt ${reconnectAttempts + 1})`)
      setReconnectAttempts((prev) => prev + 1)
    }
  }, [taskId, reconnectAttempts])

  const handleSSETaskNotFound = useCallback(() => {
    setStreamingError('Task not found')
    setIsStreaming(false)
  }, [])

  // Manage SSE connection lifecycle
  useEffect(() => {
    if (shouldStream) {
      // Implement reconnection with exponential backoff
      const connectWithRetry = () => {
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
      }

      // Connect immediately
      connectWithRetry()

      // Set up reconnection timer if needed
      let reconnectTimer: NodeJS.Timeout | undefined
      if (reconnectAttempts > 0 && reconnectAttempts < 5) {
        const delay = Math.min(1000 * 2 ** (reconnectAttempts - 1), 30000)
        reconnectTimer = setTimeout(connectWithRetry, delay)
      }

      return () => {
        console.log(`[SSE] Cleaning up streaming for task ${taskId}`)
        if (reconnectTimer) clearTimeout(reconnectTimer)
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
  }, [
    shouldStream,
    taskId,
    reconnectAttempts,
    handleSSEEvent,
    handleSSEError,
    handleSSEConnect,
    handleSSEDisconnect,
    handleSSETaskNotFound,
  ])

  // Clean up when component unmounts or taskId changes
  useEffect(() => {
    return () => {
      setStreamEvents([])
      setIsStreaming(false)
      setStreamingError(null)
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

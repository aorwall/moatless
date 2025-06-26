import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
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
  const [lastEventId, setLastEventId] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Get task metadata
  const { data: metadata } = useTask(taskId)

  // Determine if we should stream based on task status
  const shouldStream = useMemo(() => {
    // Don't stream if we don't have basic task data yet
    if (!metadata) return false

    // Stream when task is in running state
    return metadata.status === 'running'
  }, [metadata])

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEEvent = useCallback((newEvent: Event) => {
    setStreamEvents((prev) => [...prev, newEvent])
    setStreamingError(null)

    // Update last event ID for stream continuation
    if (newEvent.eventId) {
      setLastEventId(newEvent.eventId)
    }

    // Refetch task data when we receive a RESULT event
    if (newEvent.type === EventType.RESULT) {
      // Refetch task metadata to update status
      queryClient.invalidateQueries({ queryKey: queryKeys.task(taskId) })
      // Also refetch agent status
      queryClient.invalidateQueries({ queryKey: queryKeys.taskAgentStatus(taskId) })
    }
  }, [queryClient, taskId])

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
    setIsStreaming(true)
    setStreamingError(null)
  }, [])

  const handleSSEDisconnect = useCallback(() => {
    setIsStreaming(false)
  }, [])

  const handleSSETaskNotFound = useCallback(() => {
    setStreamingError('Task not found')
    setIsStreaming(false)
  }, [])

  // Manage SSE connection lifecycle
  useEffect(() => {
    if (shouldStream) {
      // Create and connect SSE client
      const client = new NDJSONSSEClient({
        taskId,
        onEvent: handleSSEEvent,
        onError: handleSSEError,
        onConnect: handleSSEConnect,
        onDisconnect: handleSSEDisconnect,
        onTaskNotFound: handleSSETaskNotFound,
      })

      // Set last event ID if available
      if (lastEventId) {
        client.lastEventId = lastEventId
      }

      sseClientRef.current = client
      client.connect()

      return () => {
        client.disconnect()
        sseClientRef.current = null
      }
    } else {
      // Clean up when not streaming
      if (sseClientRef.current) {
        sseClientRef.current.disconnect()
        sseClientRef.current = null
      }
      setIsStreaming(false)
    }
  }, [
    shouldStream,
    taskId,
    lastEventId,
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
      setLastEventId(null)
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

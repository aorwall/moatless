import type { Event } from '@shared/types/event'
import { EventType } from '@shared/types/event'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { NDJSONSSEClient } from '@/features/tasks/api/ndjson-sse'
import { useAgentStatus, useTask, useWorkspaceStatus } from '@/features/tasks/hooks/use-tasks'
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
  const { data: agentStatus } = useAgentStatus(taskId)
  const { data: workspaceStatus } = useWorkspaceStatus(taskId)

  // Determine if we should stream based on task status
  const shouldStream = useMemo(() => {
    // Don't stream if we don't have basic task data yet
    if (!metadata) return false

    // Stream when task is in running state
    if (metadata.status === 'running') return true

    // Also stream when agent is running, even if task status hasn't updated yet
    if (agentStatus?.status === 'running') return true

    // Also stream when workspace is active (executing)
    if (workspaceStatus?.status === 'executing') return true

    // Stream for open tasks that might be about to start
    if (
      metadata.status === 'open' &&
      (agentStatus?.status === 'idle' || workspaceStatus?.status === 'ready')
    )
      return true

    return false
  }, [metadata, agentStatus, workspaceStatus])

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEEvent = useCallback(
    (newEvent: Event) => {
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
  }, [taskId])

  const handleSSEDisconnect = useCallback(() => {
    console.log(`[SSE] Disconnected for task ${taskId}`)
    setIsStreaming(false)
  }, [taskId])

  const handleSSETaskNotFound = useCallback(() => {
    setStreamingError('Task not found')
    setIsStreaming(false)
  }, [])

  // Manage SSE connection lifecycle
  useEffect(() => {
    if (shouldStream) {
      console.log(`[SSE] Starting streaming for task ${taskId}`, {
        taskStatus: metadata?.status,
        agentStatus: agentStatus?.status,
        workspaceStatus: workspaceStatus?.status,
      })
      
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
        console.log(`[SSE] Cleaning up streaming for task ${taskId}`)
        client.disconnect()
        sseClientRef.current = null
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

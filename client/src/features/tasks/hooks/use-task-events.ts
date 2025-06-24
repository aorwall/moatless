import type { Event } from '@shared/types/event'
import { useMemo } from 'react'
import { useTaskStreaming } from '@/features/tasks/hooks/use-task-streaming'
import { useTaskEvents as useTaskEventsAPI } from '@/features/tasks/hooks/use-tasks'

interface UseTaskEventsResult {
  events: Array<Event>
  isLoading: boolean
  error: Error | null
  isStreaming: boolean
}

/**
 * Unified hook for task events that automatically manages streaming vs API events.
 * - When task/workspace is active, connects to SSE stream and returns stream events
 * - When task/workspace is inactive, returns regular API events
 * - Never mixes API and stream events
 * - Uses React state instead of Zustand to avoid infinite render loops
 */
export function useTaskEventsWithStreaming(taskId: string): UseTaskEventsResult {
  // Get regular events from API
  const { data: apiEvents, isLoading, error: apiError } = useTaskEventsAPI(taskId)

  // Get streaming state using React state (not Zustand)
  const { streamEvents, isStreaming, streamingError } = useTaskStreaming(taskId)

  // Choose which events to display - NEVER mix API and stream events
  const events = useMemo(() => {
    // When streaming is active, use stream events (even if empty)
    if (isStreaming) {
      return streamEvents
    }
    // When not streaming, use API events
    return apiEvents || []
  }, [isStreaming, streamEvents, apiEvents])

  // Determine final error state
  const error = useMemo(() => {
    if (isStreaming && streamingError) {
      return new Error(streamingError)
    }
    return apiError || null
  }, [isStreaming, streamingError, apiError])

  return {
    events,
    isLoading: isStreaming ? false : isLoading,
    error,
    isStreaming,
  }
}

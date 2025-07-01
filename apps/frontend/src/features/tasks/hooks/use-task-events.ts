import type { Event } from '@moatless/shared/types/event'
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
  // Start SSE connection immediately
  const { streamEvents, isStreaming, streamingError } = useTaskStreaming(taskId)

  // Get regular events from API (will be used as initial data)
  const {
    data: apiEvents,
    isLoading,
    error: apiError,
  } = useTaskEventsAPI(taskId, undefined, {
    // Don't refetch if we're streaming
    enabled: !isStreaming,
    // Refetch periodically if not streaming to catch any missed events
    refetchInterval: isStreaming ? false : 5000,
  })

  // Merge API and stream events intelligently
  const events = useMemo(() => {
    // If we have stream events, they are the source of truth
    if (streamEvents.length > 0) {
      return streamEvents
    }

    // If streaming just started but no events yet, show API events
    // This provides immediate data while waiting for stream events
    if (isStreaming && apiEvents) {
      return apiEvents
    }

    // Otherwise use API events
    return apiEvents || []
  }, [isStreaming, streamEvents, apiEvents])

  // Determine final error state
  const error = useMemo(() => {
    if (streamingError) {
      return new Error(streamingError)
    }
    return apiError || null
  }, [streamingError, apiError])

  return {
    events,
    isLoading: !isStreaming && isLoading && !apiEvents, // Only show loading if no data at all
    error,
    isStreaming,
  }
}

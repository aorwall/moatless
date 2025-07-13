import type { Event } from '@moatless/types/types/event'
import { EventType } from '@moatless/types/types/event'
import { useMemo } from 'react'

interface UseMessageCountingProps {
  events: Event[]
  isStreaming: boolean
}

export function useMessageCounting({ events, isStreaming }: UseMessageCountingProps) {
  // Memoize message counts calculation for better performance
  const messageCounts = useMemo(() => {
    const counts = new Map<number, number>()

    if (isStreaming && events.length) {
      // Count message events with role assistant for each iteration
      events.forEach((event) => {
        if (
          event.type === EventType.MESSAGE &&
          event.payload?.role === 'assistant' &&
          event.iterationId
        ) {
          const currentCount = counts.get(event.iterationId) || 0
          counts.set(event.iterationId, currentCount + 1)
        }
      })
    }

    return counts
  }, [events, isStreaming])

  // Calculate message count from events for active iterations
  const getMessageCountForIteration = (iterationIndex: number, staticCount: number): number => {
    if (!(isStreaming && events.length)) {
      return staticCount
    }

    const iterationId = iterationIndex + 1
    const dynamicCount = messageCounts.get(iterationId) || 0
    return dynamicCount || staticCount
  }

  return {
    messageCounts,
    getMessageCountForIteration,
  }
}

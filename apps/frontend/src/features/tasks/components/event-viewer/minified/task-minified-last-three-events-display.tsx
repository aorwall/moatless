import type { Event, MessageEvent } from '@moatless/shared/types/event'
import { useTaskData, useTaskEvents } from '@/features/tasks/contexts/task-providers'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { MinifiedLastThreeEventsDisplay } from './minified-last-three-events-display'

// Extract the most recent context tokens from events
function getLatestContextTokens(events: Array<Event | MergedEvent>): number | null {
  // Look for the most recent message event with usage data
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i]
    if (event.type === 'message') {
      const messageEvent = event as MessageEvent
      const payload = messageEvent.payload
      
      if (payload?.type === 'assistant' && payload.message?.usage) {
        const usage = payload.message.usage as Record<string, unknown>
        const contextTokens = Number(usage.input_tokens || 0) + Number(usage.cache_read_input_tokens || 0)
        if (contextTokens > 0) {
          return contextTokens
        }
      }
    }
  }
  return null
}

export function TaskMinifiedLastThreeEventsDisplay() {
  const { events } = useTaskEvents()
  const { status, startedAt } = useTaskData()

  if (!events || events.length === 0) {
    return null
  }

  const isRunning = status === 'running'
  const runningStartTime = startedAt && isRunning ? new Date(startedAt) : undefined
  const latestContextTokens = getLatestContextTokens(events)

  return (
    <MinifiedLastThreeEventsDisplay 
      events={events} 
      isRunning={isRunning}
      runningStartTime={runningStartTime}
      latestContextTokens={latestContextTokens}
    />
  )
}

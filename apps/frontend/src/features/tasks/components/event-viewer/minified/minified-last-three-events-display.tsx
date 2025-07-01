import type { Event, MessageEvent } from '@moatless/shared/types/event'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { IsolatedTimer } from '@/lib/components/ui/isolated-timer'
import { MinifiedEventItem } from './minified-event-item'

interface MinifiedLastThreeEventsDisplayProps {
  events: Array<Event | MergedEvent>
  className?: string
  isRunning?: boolean
  runningStartTime?: Date
}

function isMessageOrToolEvent(event: Event | MergedEvent): boolean {
  if (event.type !== 'message') return false

  const messageEvent = event as MessageEvent
  const payload = messageEvent.payload

  // Skip user messages (which are usually tool results)
  if (payload?.type === 'user') return false

  // Include assistant messages (which can be text or tool calls)
  if (payload?.type === 'assistant') return true

  return false
}

export function MinifiedLastThreeEventsDisplay({
  events,
  className = '',
  isRunning = false,
  runningStartTime,
}: MinifiedLastThreeEventsDisplayProps) {
  const [displayedEvents, setDisplayedEvents] = useState<Array<Event | MergedEvent>>([])

  // Filter and update displayed events
  useEffect(() => {
    // Filter to only include message and tool events
    const filteredEvents = events.filter(isMessageOrToolEvent)

    // Take the last 3 events
    const lastThree = filteredEvents.slice(-3)

    setDisplayedEvents(lastThree)
  }, [events])

  if (displayedEvents.length === 0) {
    return null
  }

  return (
    <div className={`relative overflow-hidden ${className}`}>
      <div className="space-y-px">
        {displayedEvents.map((event, index) => {
          const isLast = index === displayedEvents.length - 1
          const opacity = isLast ? 1 : 0.5 + index * 0.25
          const bgOpacity = isLast ? 0.5 : 0.3

          return (
            <div
              key={event.eventId}
              className="transition-all duration-300 ease-in-out rounded-md px-2 py-1.5 relative"
              style={{
                opacity,
                backgroundColor: `hsl(var(--muted) / ${bgOpacity})`,
              }}
            >
              <MinifiedEventItem event={event} />
              {isLast && isRunning && (
                <div className="absolute right-2 top-1/2 -translate-y-1/2 flex items-center gap-1.5 text-xs text-muted-foreground">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  {runningStartTime ? (
                    <IsolatedTimer startTime={runningStartTime} />
                  ) : (
                    <span>running</span>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

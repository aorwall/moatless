import type { Event, MessageEvent } from '@moatless/shared/types/event'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { Progress } from '@/lib/components/ui/progress'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'
import { MinifiedEventItem } from './minified-event-item'

interface MinifiedLastThreeEventsDisplayProps {
  events: Array<Event | MergedEvent>
  className?: string
  isRunning?: boolean
  runningStartTime?: Date
  latestContextTokens?: number | null
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
  latestContextTokens,
}: MinifiedLastThreeEventsDisplayProps) {
  const [displayedEvents, setDisplayedEvents] = useState<Array<Event | MergedEvent>>([])
  const [runningDuration, setRunningDuration] = useState<string>('')

  // Filter and update displayed events
  useEffect(() => {
    // Filter to only include message and tool events
    const filteredEvents = events.filter(isMessageOrToolEvent)

    // Take the last 3 events
    const lastThree = filteredEvents.slice(-3)

    setDisplayedEvents(lastThree)
  }, [events])

  // Update running duration every second (like chat input)
  useEffect(() => {
    if (!isRunning || !runningStartTime) {
      setRunningDuration('')
      return
    }

    const updateDuration = () => {
      setRunningDuration(formatRunningDuration(runningStartTime))
    }

    updateDuration() // Initial update
    const interval = setInterval(updateDuration, 1000)

    return () => clearInterval(interval)
  }, [isRunning, runningStartTime])

  if (displayedEvents.length === 0) {
    return null
  }

  return (
    <div className={`relative overflow-hidden ${className}`}>
      <div className="flex items-start gap-3">
        {/* Events column - takes most of the space */}
        <div className="flex-1 min-w-0 space-y-px">
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
              </div>
            )
          })}
        </div>

        {/* Status column - compact side panel when running */}
        {isRunning && (latestContextTokens || runningStartTime) && (
          <div className="flex-shrink-0">
            <div className="bg-muted/30 rounded-md px-3 py-2 h-full flex flex-col justify-center gap-2">
              <div className="flex items-center gap-2">
                <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
                <span className="text-xs font-medium text-foreground">Running</span>
              </div>
              
              {runningStartTime && runningDuration && (
                <div className="flex items-center gap-2 text-xs">
                  <span className="font-mono font-medium text-foreground">
                    {runningDuration}
                  </span>
                </div>
              )}
              
              {latestContextTokens && (
                <div className="flex flex-col gap-1 text-xs">
                  <span className="font-mono font-medium text-foreground">
                    {latestContextTokens > 1000 
                      ? `${Math.round(latestContextTokens / 1000)}k tokens`
                      : `${latestContextTokens.toLocaleString()} tokens`
                    }
                  </span>
                  <Progress 
                    value={(latestContextTokens / 200000) * 100} 
                    className="w-full h-1.5" 
                  />
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

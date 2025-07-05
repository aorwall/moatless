import type { Event } from '@moatless/shared/types/event'
import { Loader2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { MinifiedEventList } from './minified-event-list'

interface MinifiedLastEventDisplayProps {
  events: Array<Event | MergedEvent>
  className?: string
  showTransitionIndicator?: boolean
  isRunning?: boolean
}

export function MinifiedLastEventDisplay({
  events,
  className = '',
  showTransitionIndicator = true,
  isRunning = false,
}: MinifiedLastEventDisplayProps) {
  const [displayedEvent, setDisplayedEvent] = useState<Event | MergedEvent | null>(null)
  const [isTransitioning, setIsTransitioning] = useState(false)
  const [fadeState, setFadeState] = useState<'in' | 'out'>('in')
  const transitionTimerRef = useRef<NodeJS.Timeout | null>(null)
  const lastEventTimeRef = useRef<number>(Date.now())

  const lastEvent = events[events.length - 1] || null

  useEffect(() => {
    // Clear any existing timer
    if (transitionTimerRef.current) {
      clearTimeout(transitionTimerRef.current)
    }

    // If there's a new event
    if (lastEvent && lastEvent.eventId !== displayedEvent?.eventId) {
      const currentTime = Date.now()
      const timeSinceLastEvent = currentTime - lastEventTimeRef.current
      lastEventTimeRef.current = currentTime

      // Check if this might be a quick tool result (merged event arriving soon after tool call)
      const isQuickTransition = timeSinceLastEvent < 500 && 'toolResult' in lastEvent

      if (isQuickTransition) {
        // Skip animation for quick tool results
        setDisplayedEvent(lastEvent)
        setFadeState('in')
        setIsTransitioning(false)
      } else {
        // Normal transition with animation
        setIsTransitioning(true)
        setFadeState('out')

        // Delay the actual event update to allow exit animation
        transitionTimerRef.current = setTimeout(() => {
          setDisplayedEvent(lastEvent)
          setFadeState('in')

          // End transition indicator after animation completes
          setTimeout(() => {
            setIsTransitioning(false)
          }, 200)
        }, 150)
      }
    }

    return () => {
      if (transitionTimerRef.current) {
        clearTimeout(transitionTimerRef.current)
      }
    }
  }, [lastEvent, displayedEvent])

  if (!displayedEvent) {
    return null
  }

  return (
    <div className={`relative overflow-hidden ${className}`}>
      <div
        className={`w-full transition-all duration-200 ease-in-out ${
          fadeState === 'in' ? 'opacity-100 translate-y-0' : 'opacity-0 -translate-y-4'
        }`}
      >
        <div className="bg-muted/50 rounded-md p-3 flex items-center gap-3">
          <div className="flex-1">
            <MinifiedEventList events={[displayedEvent]} />
          </div>
          {isRunning && (
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" />
              <span>running</span>
            </div>
          )}
        </div>
      </div>

      {/* Pulse indicator when transitioning */}
      {showTransitionIndicator && isTransitioning && (
        <div className="absolute top-2 right-2 w-2 h-2 bg-blue-500 rounded-full animate-pulse" />
      )}
    </div>
  )
}

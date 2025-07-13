import type { Event } from '@moatless/types/types/event'
import { useEffect, useRef, useState } from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { EventList } from './event-list'

interface LastEventDisplayProps {
  events: Array<Event | MergedEvent>
  className?: string
  showTransitionIndicator?: boolean
}

export function LastEventDisplay({
  events,
  className = '',
  showTransitionIndicator = true,
}: LastEventDisplayProps) {
  const [displayedEvent, setDisplayedEvent] = useState<Event | MergedEvent | null>(null)
  const [isTransitioning, setIsTransitioning] = useState(false)
  const [fadeState, setFadeState] = useState<'in' | 'out'>('in')
  const transitionTimerRef = useRef<NodeJS.Timeout | null>(null)

  const lastEvent = events[events.length - 1] || null

  useEffect(() => {
    // Clear any existing timer
    if (transitionTimerRef.current) {
      clearTimeout(transitionTimerRef.current)
    }

    // If there's a new event, start transition
    if (lastEvent && lastEvent.eventId !== displayedEvent?.eventId) {
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
        <EventList events={[displayedEvent]} />
      </div>

      {/* Pulse indicator when transitioning */}
      {showTransitionIndicator && isTransitioning && (
        <div className="absolute top-2 right-2 w-2 h-2 bg-blue-500 rounded-full animate-pulse" />
      )}
    </div>
  )
}

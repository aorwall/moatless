import type { Event } from '@/lib/types/events'
import { MinifiedEventItem } from './minified-event-item.tsx'

interface MinifiedEventListProps {
  events: Event[]
  className?: string
}

export function MinifiedEventList({ events, className = '' }: MinifiedEventListProps) {
  if (!events || events.length === 0) {
    return null
  }

  return (
    <div className={`space-y-2 ${className}`}>
      {events.map((event) => (
        <MinifiedEventItem key={event.eventId} event={event} />
      ))}
    </div>
  )
}

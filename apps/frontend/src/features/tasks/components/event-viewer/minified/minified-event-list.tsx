import type { Event } from '@moatless/shared/types/event'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { MinifiedEventItem } from './minified-event-item'

interface MinifiedEventListProps {
  events: Array<Event | MergedEvent>
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

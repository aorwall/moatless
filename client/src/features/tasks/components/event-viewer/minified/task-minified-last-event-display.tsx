import { useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { MinifiedEventItem } from './minified-event-item'

export function TaskMinifiedLastEventDisplay() {
  const { events } = useTaskEvents()

  if (!events || events.length === 0) {
    return null
  }

  const lastEvent = events[events.length - 1]
  
  return <MinifiedEventItem event={lastEvent} />
}

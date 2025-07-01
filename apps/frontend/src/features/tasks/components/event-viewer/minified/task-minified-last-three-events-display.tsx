import { useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { MinifiedLastThreeEventsDisplay } from './minified-last-three-events-display'

export function TaskMinifiedLastThreeEventsDisplay() {
  const { events } = useTaskEvents()

  if (!events || events.length === 0) {
    return null
  }

  return <MinifiedLastThreeEventsDisplay events={events} />
}

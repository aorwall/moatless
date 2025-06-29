import { useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { EventViewer } from './event-viewer'

export function TaskLastEventDisplay() {
  const { events } = useTaskEvents()

  if (!events || events.length === 0) {
    return null
  }

  return <EventViewer />
}

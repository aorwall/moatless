import { useLatestContextTokens, useTaskDuration, useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { MinifiedLastThreeEventsDisplay } from './minified-last-three-events-display'

export function TaskMinifiedLastThreeEventsDisplay() {
  const { events } = useTaskEvents()
  const { isRunning, runningStartTime } = useTaskDuration()
  const latestContextTokens = useLatestContextTokens()

  if (!events || events.length === 0) {
    return null
  }

  return (
    <MinifiedLastThreeEventsDisplay 
      events={events} 
      isRunning={isRunning}
      runningStartTime={runningStartTime}
      latestContextTokens={latestContextTokens > 0 ? latestContextTokens : null}
    />
  )
}

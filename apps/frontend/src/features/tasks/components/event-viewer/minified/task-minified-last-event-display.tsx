import { useLatestContextTokens, useTaskDuration, useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { MinifiedLastEventDisplay } from './minified-last-event-display'

export function TaskMinifiedLastEventDisplay() {
  const { events } = useTaskEvents()
  const { isRunning, runningStartTime } = useTaskDuration()
  const latestContextTokens = useLatestContextTokens()

  if (!events || events.length === 0) {
    return null
  }

  return (
    <MinifiedLastEventDisplay 
      events={events} 
      isRunning={isRunning}
      runningStartTime={runningStartTime}
      latestContextTokens={latestContextTokens > 0 ? latestContextTokens : null}
    />
  )
}

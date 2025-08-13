import {
  useIsAgentRunning,
  useLatestContextTokens,
  useTaskEvents,
} from '@/features/tasks/contexts/task-providers.tsx'
import { MinifiedLastEventsDisplay } from './minified-last-events-display.tsx'

export function TaskMinifiedLastEventsDisplay() {
  const { events } = useTaskEvents()
  const isRunning = useIsAgentRunning()
  const latestContextTokens = useLatestContextTokens()

  if (!events || events.length === 0) {
    return null
  }

  return (
    <MinifiedLastEventsDisplay
      events={events}
      isRunning={isRunning}
      runningStartTime={undefined}
      latestContextTokens={latestContextTokens > 0 ? latestContextTokens : null}
    />
  )
}

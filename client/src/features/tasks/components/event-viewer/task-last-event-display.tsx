import { useTaskDataContext } from '@/features/tasks/contexts/task-data-context'
import { mergeToolEvents } from '@/features/tasks/utils/merge-tool-events'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { LastEventDisplay } from './last-event-display'

interface TaskLastEventDisplayProps {
  taskId: string
  className?: string
  showTransitionIndicator?: boolean
}

export function TaskLastEventDisplay({
  taskId,
  className,
  showTransitionIndicator,
}: TaskLastEventDisplayProps) {
  const { events, isLoading, error, isStreaming } = useTaskDataContext()

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading && !isStreaming) {
    return (
      <div className={`animate-pulse ${className}`}>
        <div className="h-20 bg-muted rounded-lg" />
      </div>
    )
  }

  if (!events || events.length === 0) {
    return null
  }

  // Merge tool events to show tool calls with their results
  const mergedEvents = mergeToolEvents(events)

  return (
    <LastEventDisplay
      events={mergedEvents}
      className={className}
      showTransitionIndicator={showTransitionIndicator}
    />
  )
}

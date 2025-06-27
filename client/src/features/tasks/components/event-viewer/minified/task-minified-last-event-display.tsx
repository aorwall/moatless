import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events'
import { mergeToolEvents } from '@/features/tasks/utils/merge-tool-events'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { MinifiedLastEventDisplay } from './minified-last-event-display'

interface TaskMinifiedLastEventDisplayProps {
  taskId: string
  className?: string
  showTransitionIndicator?: boolean
  isRunning?: boolean
}

export function TaskMinifiedLastEventDisplay({
  taskId,
  className,
  showTransitionIndicator,
  isRunning,
}: TaskMinifiedLastEventDisplayProps) {
  const { events, isLoading, error, isStreaming } = useTaskEventsWithStreaming(taskId)

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading && !isStreaming) {
    return (
      <div className={`animate-pulse ${className}`}>
        <div className="h-12 bg-muted rounded-md" />
      </div>
    )
  }

  if (!events || events.length === 0) {
    return null
  }

  // Merge tool events to show tool calls with their results
  const mergedEvents = mergeToolEvents(events)

  return (
    <MinifiedLastEventDisplay
      events={mergedEvents}
      className={className}
      showTransitionIndicator={showTransitionIndicator}
      isRunning={isRunning}
    />
  )
}

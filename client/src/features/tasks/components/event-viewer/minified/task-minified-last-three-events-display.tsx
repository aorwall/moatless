import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events'
import { useTask } from '@/features/tasks/hooks/use-tasks'
import { mergeToolEvents } from '@/features/tasks/utils/merge-tool-events'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { MinifiedLastThreeEventsDisplay } from './minified-last-three-events-display'

interface TaskMinifiedLastThreeEventsDisplayProps {
  taskId: string
  className?: string
}

export function TaskMinifiedLastThreeEventsDisplay({
  taskId,
  className,
}: TaskMinifiedLastThreeEventsDisplayProps) {
  const { data: task } = useTask(taskId)
  const { events, isLoading, error, isStreaming } = useTaskEventsWithStreaming(taskId)

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading && !isStreaming) {
    return (
      <div className={`animate-pulse ${className}`}>
        <div className="h-24 bg-muted rounded-md" />
      </div>
    )
  }

  if (!events || events.length === 0) {
    return null
  }

  // Get the last iteration start time for running tasks
  let runningStartTime: Date | undefined
  if (task?.status === 'running' && task.iterations?.length > 0) {
    const lastIteration = task.iterations[task.iterations.length - 1]
    runningStartTime = new Date(lastIteration.startTime)
  }

  // Merge tool events to show tool calls with their results
  const mergedEvents = mergeToolEvents(events)

  return (
    <MinifiedLastThreeEventsDisplay
      events={mergedEvents}
      className={className}
      isRunning={task?.status === 'running'}
      runningStartTime={runningStartTime}
    />
  )
}

import type { Task } from '@moatless/types/types/task'
import { Link } from '@tanstack/react-router'
import { MinifiedEventList } from '@/features/tasks/components/minified-event-viewer/minified-event-list'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTaskEvents } from '@/features/tasks/hooks/use-tasks'
import { Card, CardContent, CardHeader } from '@/lib/components/ui/card'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'

function getRelativeTime(date: Date | string): string {
  const now = new Date()
  const then = new Date(date)
  const seconds = Math.floor((now.getTime() - then.getTime()) / 1000)

  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  const weeks = Math.floor(days / 7)
  if (weeks < 4) return `${weeks}w ago`
  const months = Math.floor(days / 30)
  if (months < 12) return `${months}mo ago`
  const years = Math.floor(days / 365)
  return `${years}y ago`
}

interface TaskCardProps {
  task: Task
  repositoryName: string
}

export function TaskCard({ task, repositoryName }: TaskCardProps) {
  // Get task title from first iteration's initial message
  const taskTitle = task.iterations?.[0]?.initialMessage?.split('\n')[0] || task.agentType

  // Get last iteration for result display
  const lastIteration = task.iterations?.[task.iterations.length - 1]

  // Get events for running tasks
  const { data: eventsData } = useTaskEvents(task.taskId, undefined, {
    enabled: task.status === 'running',
    refetchInterval: false, // Real-time updates handled by global event stream
  })

  return (
    <Card className="cursor-pointer transition-all hover:-translate-y-0.5 hover:shadow-md">
      <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
        <CardHeader className="pb-3">
          <div className="flex items-start gap-2 min-w-0">
            <div className="flex-shrink-0">
              <TaskStatusIcon task={task} showText={false} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-1">
              <h3 className="font-medium text-sm leading-tight line-clamp-2">{taskTitle}</h3>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className="truncate">{repositoryName}</span>
                <span className="shrink-0">
                  {task.status === 'running' && lastIteration?.startTime
                    ? formatRunningDuration(lastIteration.startTime)
                    : getRelativeTime(task.lastMessageAt || task.createdAt)}
                </span>
              </div>
            </div>
          </div>
        </CardHeader>

        <CardContent className="pt-0">
          {task.status === 'running' ? (
            // Show events for running tasks
            eventsData && eventsData.length > 0 ? (
              <div className="min-h-[80px] max-h-[120px] overflow-y-auto">
                <MinifiedEventList events={eventsData} />
              </div>
            ) : (
              <div className="text-sm text-muted-foreground min-h-[80px] flex items-center">
                Starting task...
              </div>
            )
          ) : (
            // Show result for completed tasks
            <div className="min-h-[80px]">
              {task.status === 'merged' && lastIteration?.gitChanges ? (
                // Special display for merged tasks with git changes
                <div className="space-y-2">
                  <p className="text-sm text-muted-foreground line-clamp-2">
                    {lastIteration.result || 'Task completed successfully'}
                  </p>
                  <div className="flex items-center gap-3 text-xs">
                    <span className="text-green-600">+{lastIteration.gitChanges.additions}</span>
                    <span className="text-red-600">-{lastIteration.gitChanges.deletions}</span>
                    <span className="text-orange-600">
                      ~{lastIteration.gitChanges.modifications}
                    </span>
                  </div>
                </div>
              ) : (
                // Default: show last result
                <p className="text-sm text-muted-foreground line-clamp-4">
                  {lastIteration?.result || lastIteration?.error?.message || 'No results yet'}
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Link>
    </Card>
  )
}

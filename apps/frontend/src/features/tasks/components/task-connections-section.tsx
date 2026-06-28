import { ConnectionChip, formatTimestamp, statusBadgeClass } from '@/features/task-connections'
import { useTaskConnectionsQuery } from '@/features/tasks/hooks/use-task-queries'
import { Badge } from '@/lib/components/ui/badge'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'

interface TaskConnectionsSectionProps {
  taskId: string
}

/**
 * Lists a task's external connections (Linear issues, GitHub PRs, Slack
 * threads) with a clickable label, live external status, message tally, and
 * last-activity time. Reuses the admin task-connections rendering helpers so
 * the panel and the admin table stay visually consistent.
 */
export function TaskConnectionsSection({ taskId }: TaskConnectionsSectionProps) {
  const { data: connections, isLoading } = useTaskConnectionsQuery(taskId)

  if (isLoading) {
    return (
      <div className="space-y-2 px-2 py-2">
        <Skeleton className="h-7 w-full" />
        <Skeleton className="h-7 w-3/4" />
      </div>
    )
  }

  if (!connections || connections.length === 0) {
    return <div className="px-2 py-3 text-xs text-muted-foreground">No external connections</div>
  }

  return (
    <div className="space-y-2 px-2 py-2 text-xs">
      {connections.map((connection) => (
        <div key={connection.bindingId} className="space-y-1">
          <div className="flex items-center justify-between gap-2">
            <ConnectionChip connection={connection} />
            {connection.status ? (
              <Badge className={cn('font-normal', statusBadgeClass(connection.stateKind))}>
                {connection.status}
              </Badge>
            ) : (
              <span className="text-muted-foreground">unknown</span>
            )}
          </div>
          <div className="flex items-center justify-between gap-2 text-muted-foreground">
            <span className="flex items-center gap-2 tabular-nums">
              <span>↓{connection.incoming}</span>
              <span>↑{connection.outgoing}</span>
            </span>
            <span>{formatTimestamp(connection.lastEventAt)}</span>
          </div>
        </div>
      ))}
    </div>
  )
}

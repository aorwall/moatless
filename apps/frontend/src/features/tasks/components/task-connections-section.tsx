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
 * threads) one per row — a clickable label, live external status, message
 * tally, and last-activity time.
 */
export function TaskConnectionsSection({ taskId }: TaskConnectionsSectionProps) {
  const { data: connections, isLoading } = useTaskConnectionsQuery(taskId)

  if (isLoading) {
    return (
      <div className="space-y-2 px-3 py-3">
        <Skeleton className="h-7 w-full" />
        <Skeleton className="h-7 w-3/4" />
      </div>
    )
  }

  if (!connections || connections.length === 0) {
    return <div className="px-3 py-3 text-sm text-muted-foreground">No external connections</div>
  }

  return (
    <div className="divide-y px-3">
      {connections.map((connection) => (
        <div
          key={connection.bindingId}
          className="flex items-center justify-between gap-3 py-2 text-sm"
        >
          <div className="min-w-0 flex-1 overflow-hidden">
            <ConnectionChip connection={connection} />
          </div>
          <div className="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
            {connection.status ? (
              <Badge className={cn('font-normal', statusBadgeClass(connection.stateKind))}>
                {connection.status}
              </Badge>
            ) : (
              <span>unknown</span>
            )}
            <span className="flex items-center gap-1.5 tabular-nums">
              <span>↓{connection.incoming}</span>
              <span>↑{connection.outgoing}</span>
            </span>
            <span className="whitespace-nowrap">{formatTimestamp(connection.lastEventAt)}</span>
          </div>
        </div>
      ))}
    </div>
  )
}

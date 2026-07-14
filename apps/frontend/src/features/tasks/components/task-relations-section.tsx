import type { TaskResponse } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { CornerUpLeft, GitBranch } from 'lucide-react'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTask, useTasksList } from '@/features/tasks/hooks/use-tasks'
import { Skeleton } from '@/lib/components/ui/skeleton'

interface TaskRelationsSectionProps {
  task: TaskResponse
}

/** Derive a short, human-readable title for a task row. */
function taskTitle(task: Pick<TaskResponse, 'name' | 'description'>): string {
  return task.name || task.description?.split('\n')[0] || 'Untitled'
}

/** A single clickable related-task row: status icon + title, links to the task. */
function RelationRow({ task }: { task: TaskResponse }) {
  return (
    <Link
      to="/tasks/$task-id"
      params={{ 'task-id': task.taskId }}
      className="flex items-center gap-2 py-2 text-sm hover:underline"
    >
      <TaskStatusIcon task={task} showText={false} className="shrink-0" />
      <span className="truncate">{taskTitle(task)}</span>
    </Link>
  )
}

/**
 * Shows how the current task relates to others in the control panel: the task
 * that spawned it (its parent) and any tasks it spawned (its children). The
 * parent is derived from `task.parentTaskId` (the `created_by` relation);
 * children are fetched by filtering the task list on `parentTaskId`. Renders
 * nothing when the task has neither a parent nor children, so unrelated tasks
 * don't get an empty section.
 */
export function TaskRelationsSection({ task }: TaskRelationsSectionProps) {
  const parentTaskId = task.parentTaskId ?? undefined

  const parentQuery = useTask(parentTaskId ?? '', { enabled: !!parentTaskId })
  const childrenQuery = useTasksList({ parentTaskId: task.taskId, limit: 50 })

  const parent = parentTaskId ? parentQuery.data : undefined
  const children = childrenQuery.data?.items ?? []
  const childrenError = childrenQuery.error

  const isLoading = (!!parentTaskId && parentQuery.isLoading) || childrenQuery.isLoading

  // Hide the section entirely only when there's genuinely nothing to show: no
  // parent, no children, not loading, and no error. A failed children query
  // keeps the section visible so the error is surfaced rather than looking like
  // "no children".
  if (!(parentTaskId || isLoading || childrenError) && children.length === 0) {
    return null
  }

  return (
    <section className="rounded-md border">
      <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">Relations</div>
      <div className="px-3 py-3 space-y-3">
        {isLoading ? (
          <Skeleton className="h-7 w-full" />
        ) : (
          <>
            {parentTaskId && (
              <div>
                <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                  <CornerUpLeft className="h-3 w-3" />
                  Parent task
                </div>
                {parent ? (
                  <RelationRow task={parent} />
                ) : (
                  <p className="py-2 text-sm text-muted-foreground">Parent task unavailable</p>
                )}
              </div>
            )}

            {childrenError ? (
              <div>
                <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                  <GitBranch className="h-3 w-3" />
                  Child tasks
                </div>
                <p className="py-2 text-sm text-destructive">Failed to load child tasks</p>
              </div>
            ) : (
              children.length > 0 && (
                <div>
                  <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                    <GitBranch className="h-3 w-3" />
                    Child tasks ({children.length})
                  </div>
                  <div className="divide-y">
                    {children.map((child) => (
                      <RelationRow key={child.taskId} task={child} />
                    ))}
                  </div>
                </div>
              )
            )}
          </>
        )}
      </div>
    </section>
  )
}

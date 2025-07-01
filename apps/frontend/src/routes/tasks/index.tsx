import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useGlobalEventStream } from '@/features/events'
import { TaskList } from '@/features/tasks/components/task-list'
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { ErrorAlert } from '@/lib/components/ui/error-alert'

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
})

function TasksIndex() {
  const queryClient = useQueryClient()
  const { subscribeToAll } = useGlobalEventStream()

  // Fetch all tasks
  const { data: tasks, isLoading, error } = useTasksList()

  // Subscribe to global events to invalidate the task list when relevant events occur
  useEffect(() => {
    return subscribeToAll((event) => {
      // Invalidate task list on relevant events
      if (event.subType && (
        event.subType === 'task:created' ||
        event.subType === 'task:updated' ||
        event.subType === 'task:statusChanged' ||
        event.subType === 'iteration:created' ||
        event.subType === 'iteration:updated'
      )) {
        queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() })
      }
    })
  }, [subscribeToAll, queryClient])

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">All Tasks</h1>
          <p className="text-muted-foreground mt-2">
            View and manage all your active and finished tasks
          </p>
        </div>
        {error ? (
          <ErrorAlert error={error} title="Failed to load tasks" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <TaskList tasks={tasks || []} loading={false} />
        )}
      </div>
    </div>
  )
}

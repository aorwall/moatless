import { createFileRoute } from '@tanstack/react-router'

import { TaskList } from '@/features/tasks/components/task-list'
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import type { Task } from '@shared/types/task'

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
})

function TasksIndex() {
  // Fetch all tasks
  const {
    data: tasks,
    isLoading,
    error,
  } = useTasksList(
    {},
    {
      refetchInterval: 5000,
      refetchIntervalInBackground: true,
    },
  )

  const activeTasks = tasks?.filter((task) => task.status === 'active') || []

  const finishedTasks = tasks?.filter((task) => task.status !== 'active') || []

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">All Tasks</h1>
          <p className="text-muted-foreground mt-2">
            View and manage all your active and archived tasks
          </p>
        </div>
        {error ? (
          <ErrorAlert error={error} title="Failed to load tasks" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <TaskList activeTasks={activeTasks} archivedTasks={finishedTasks} loading={false} />
        )}
      </div>
    </div>
  )
}

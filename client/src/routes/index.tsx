import { createFileRoute } from '@tanstack/react-router'

import { CreateTask } from '@/features/tasks/components/create-task'
import { TaskList } from '@/features/tasks/components/task-list'
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { ErrorAlert } from '@/lib/components/ui/error-alert'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  // Fetch all tasks (both active and finished)
  const {
    data: tasks,
    isLoading,
    error,
  } = useTasksList(
    {}, // Remove activeOnly filter to get all tasks
    {
      refetchInterval: 5000, // Poll every 5 seconds
      refetchIntervalInBackground: true,
    },
  )

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl space-y-8">
        <CreateTask />
        {error ? (
          <ErrorAlert error={error} title="Failed to load data" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <TaskList tasks={tasks || []} loading={false} />
        )}
      </div>
    </div>
  )
}

import { createFileRoute } from '@tanstack/react-router'
import { LazyFileOnlyViewer } from '@/features/files'
import { TaskProviders } from '@/features/tasks'
import { taskDetailQueryOptions } from '@/features/tasks/hooks/use-task-queries'
import { taskSearchSchema } from '@/features/tasks/types/task-search-schema'
import { queryClient } from '@/lib/api/query-client'

export const Route = createFileRoute('/tasks/$task-id/file')({
  validateSearch: taskSearchSchema,
  loader: ({ params }) =>
    queryClient.ensureQueryData(taskDetailQueryOptions(params['task-id'])).catch(() => {
      /* ignore: file viewer owns file loading/error state */
    }),
  component: TaskFileRoute,
})

function TaskFileRoute() {
  const { 'task-id': taskId } = Route.useParams()

  return (
    <div className="h-dvh w-full overflow-hidden">
      <TaskProviders taskId={taskId}>
        <LazyFileOnlyViewer taskId={taskId} />
      </TaskProviders>
    </div>
  )
}

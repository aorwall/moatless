import { createFileRoute } from '@tanstack/react-router'
import { AppSidebar } from '@/features/navigation/components/app-sidebar'
import { TaskPage, TaskProviders } from '@/features/tasks'
import { tasksApi } from '@/features/tasks/api/tasks'
import { useMarkTaskReadOnView } from '@/features/tasks/hooks/use-tasks'
import { taskSearchSchema } from '@/features/tasks/types/task-search-schema'
import { queryClient } from '@/lib/api/query-client'
import { queryKeys } from '@/lib/api/query-keys'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

export const Route = createFileRoute('/tasks/$task-id/')({
  validateSearch: taskSearchSchema,
  // Prefetch the task into the query cache before (or on hover ahead of)
  // navigation, so the page renders with data instead of a skeleton.
  // `ensureQueryData` is a no-op when the cache already has fresh data.
  // Errors are deliberately not thrown here: the component's own query
  // owns error handling (FullPageError), not the router's error screen.
  loader: ({ params }) =>
    queryClient
      .ensureQueryData({
        queryKey: queryKeys.tasks.detail(params['task-id']),
        queryFn: () => tasksApi.getTask(params['task-id']),
      })
      .catch(() => {
        /* ignore: component's own query owns error handling via FullPageError */
      }),
  component: TaskPageRoute,
})

function TaskPageRoute() {
  const { 'task-id': taskId } = Route.useParams()

  // Clear the unread indicator while this task is open (and on navigating away).
  useMarkTaskReadOnView(taskId)

  return (
    <SidebarProvider defaultOpen={false}>
      <div className="flex h-full w-full">
        <AppSidebar />
        <div className="flex-1 h-full overflow-hidden">
          <TaskProviders taskId={taskId}>
            <TaskPage />
          </TaskProviders>
        </div>
      </div>
    </SidebarProvider>
  )
}

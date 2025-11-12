import { createFileRoute } from '@tanstack/react-router'
import { AppSidebar } from '@/features/navigation/components/app-sidebar'
import { TaskPage, TaskProviders } from '@/features/tasks'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

export const Route = createFileRoute('/tasks/$task-id/')({
  component: TaskDetail,
})

function TaskDetail() {
  const { 'task-id': taskId } = Route.useParams()

  return (
    <SidebarProvider defaultOpen={false}>
      <div className="flex h-dvh w-full">
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

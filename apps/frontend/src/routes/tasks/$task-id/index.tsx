import { createFileRoute } from '@tanstack/react-router'
import { z } from 'zod'
import { AppSidebar } from '@/features/navigation/components/app-sidebar'
import { TaskPage, TaskProviders } from '@/features/tasks'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

/**
 * Search params for task detail page.
 * All params are optional and stored in URL for shareability.
 *
 * @property view - Current view: chat/files/preview
 * @property file - Selected file path in file explorer
 * @property path - Current path in preview (e.g., /about)
 * @property viewport - Preview viewport mode: mobile/tablet/desktop
 */
const taskSearchSchema = z.object({
  view: z.enum(['chat', 'files', 'preview']).optional(),
  file: z.string().optional(),
  path: z.string().optional(),
  viewport: z.enum(['mobile', 'tablet', 'desktop']).optional(),
})

export type TaskSearchParams = z.infer<typeof taskSearchSchema>

export const Route = createFileRoute('/tasks/$task-id/')({
  validateSearch: taskSearchSchema,
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

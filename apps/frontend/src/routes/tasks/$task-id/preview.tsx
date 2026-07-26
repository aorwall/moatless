import { createFileRoute } from '@tanstack/react-router'
import { StandalonePreview } from '@/features/files'
import { taskSearchSchema } from '@/features/tasks/types/task-search-schema'

export const Route = createFileRoute('/tasks/$task-id/preview')({
  validateSearch: taskSearchSchema,
  component: TaskPreviewRoute,
})

function TaskPreviewRoute() {
  const { 'task-id': taskId } = Route.useParams()

  return (
    <div className="h-dvh w-full overflow-hidden">
      <StandalonePreview taskId={taskId} />
    </div>
  )
}

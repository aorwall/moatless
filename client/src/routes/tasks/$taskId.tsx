import { createFileRoute } from '@tanstack/react-router'
import { TaskPage, TaskProviders } from '@/features/tasks'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
})

function TaskDetail() {
  const { taskId } = Route.useParams()

  return (
    <TaskProviders taskId={taskId}>
      <TaskPage />
    </TaskProviders>
  )
}

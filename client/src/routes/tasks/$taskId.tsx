import { createFileRoute } from '@tanstack/react-router'
import { TaskPage, TaskProvider } from '@/features/tasks'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
})

function TaskDetail() {
  const { taskId } = Route.useParams()

  // TaskProvider handles all data fetching and loading/error states
  return (
    <TaskProvider taskId={taskId}>
      <TaskPage />
    </TaskProvider>
  )
}

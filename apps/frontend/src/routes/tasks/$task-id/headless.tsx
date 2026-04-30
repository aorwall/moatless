import { createFileRoute } from '@tanstack/react-router'
import { TaskPage, TaskProviders } from '@/features/tasks'
import { taskSearchSchema } from '@/features/tasks/types/task-search-schema'

export const Route = createFileRoute('/tasks/$task-id/headless')({
  validateSearch: taskSearchSchema,
  component: TaskHeadless,
})

function TaskHeadless() {
  const { 'task-id': taskId } = Route.useParams()

  return (
    <div className="flex h-dvh w-full">
      <TaskProviders taskId={taskId}>
        <TaskPage headless />
      </TaskProviders>
    </div>
  )
}

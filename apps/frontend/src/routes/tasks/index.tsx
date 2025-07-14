import { createFileRoute } from '@tanstack/react-router'
import { TaskDashboard } from '@/features/tasks/components/dashboard'

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
})

function TasksIndex() {
  return <TaskDashboard />
}

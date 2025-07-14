import { createFileRoute } from '@tanstack/react-router'
import { TaskDashboard } from '@/features/tasks/components/dashboard'

export const Route = createFileRoute('/dashboard')({
  component: Dashboard,
})

function Dashboard() {
  return <TaskDashboard />
}

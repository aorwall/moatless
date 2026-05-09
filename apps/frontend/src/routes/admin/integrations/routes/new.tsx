import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { AdminPage } from '@/features/admin'
import { TaskRouteForm } from '@/features/integrations/components/task-route-form'

export const Route = createFileRoute('/admin/integrations/routes/new')({
  component: NewRoutePage,
})

function NewRoutePage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/admin/integrations/routes' })

  return (
    <AdminPage
      title="New Route"
      description="Create a reusable route for triggers."
      backTo={{ to: '/admin/integrations/routes', label: 'Back to routes' }}
    >
      <TaskRouteForm route={null} onSuccess={goBack} onCancel={goBack} />
    </AdminPage>
  )
}

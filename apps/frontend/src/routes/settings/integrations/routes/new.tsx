import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { SettingsPage } from '@/features/settings/components/settings-page'
import { TaskRouteForm } from '@/features/settings/components/task-route-form'

export const Route = createFileRoute('/settings/integrations/routes/new')({
  component: NewRoutePage,
})

function NewRoutePage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/settings/integrations/routes' })

  return (
    <SettingsPage
      title="New Route"
      description="Create a reusable route for triggers."
      backTo={{ to: '/settings/integrations/routes', label: 'Back to routes' }}
    >
      <TaskRouteForm route={null} onSuccess={goBack} onCancel={goBack} />
    </SettingsPage>
  )
}

import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { SettingsPage } from '@/features/settings/components/settings-page'
import { TaskTriggerForm } from '@/features/settings/components/task-trigger-form'

export const Route = createFileRoute('/settings/integrations/triggers/new')({
  component: NewTriggerPage,
})

function NewTriggerPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/settings/integrations/triggers' })

  return (
    <SettingsPage
      title="New Trigger"
      description="Create an integration trigger."
      backTo={{ to: '/settings/integrations/triggers', label: 'Back to triggers' }}
    >
      <TaskTriggerForm trigger={null} onSuccess={goBack} onCancel={goBack} />
    </SettingsPage>
  )
}

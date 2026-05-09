import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { AdminPage } from '@/features/admin'
import { TaskTriggerForm } from '@/features/integrations/components/task-trigger-form'

export const Route = createFileRoute('/admin/integrations/triggers/new')({
  component: NewTriggerPage,
})

function NewTriggerPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/admin/integrations/triggers' })

  return (
    <AdminPage
      title="New Trigger"
      description="Create an integration trigger."
      backTo={{ to: '/admin/integrations/triggers', label: 'Back to triggers' }}
    >
      <TaskTriggerForm trigger={null} onSuccess={goBack} onCancel={goBack} />
    </AdminPage>
  )
}

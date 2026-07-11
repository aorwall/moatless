import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { AdminPage } from '@/features/admin'
import { TaskTriggerForm } from '@/features/integrations/components/task-trigger-form'

export const Route = createLazyFileRoute('/admin/triggers/new')({
  component: NewTriggerPage,
})

function NewTriggerPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/admin/triggers' })

  return (
    <AdminPage
      title="New Trigger"
      description="Create an adapter-event or schedule trigger with its embedded rule."
      backTo={{ to: '/admin/triggers', label: 'Back to triggers' }}
    >
      <TaskTriggerForm trigger={null} onSuccess={goBack} onCancel={goBack} />
    </AdminPage>
  )
}

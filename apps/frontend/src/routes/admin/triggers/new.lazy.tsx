import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { TaskTriggerForm } from '@/features/integrations/components/task-trigger-form'

export const Route = createLazyFileRoute('/admin/triggers/new')({
  component: NewTriggerPage,
})

function NewTriggerPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/admin/triggers' })

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">New Trigger</h1>
        <p className="text-sm text-muted-foreground">
          Create an adapter-event or schedule trigger with its embedded rule.
        </p>
      </div>
      <TaskTriggerForm trigger={null} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}

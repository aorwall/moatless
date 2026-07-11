import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { AdminPage } from '@/features/admin'
import { TaskTriggerForm } from '@/features/integrations/components/task-trigger-form'
import { useTrigger } from '@/features/integrations/hooks/use-integrations'

export const Route = createLazyFileRoute('/admin/triggers/$trigger-id/edit')({
  component: EditTriggerPage,
})

function EditTriggerPage() {
  const params = Route.useParams()
  const triggerId = params['trigger-id']
  const navigate = useNavigate()
  const { data: trigger, isLoading } = useTrigger(triggerId)

  const goBack = () =>
    navigate({
      to: '/admin/triggers/$trigger-id',
      params: { 'trigger-id': triggerId },
    })

  return (
    <AdminPage
      title="Edit Trigger"
      description="Update trigger source and rule fields."
      backTo={{
        to: '/admin/triggers/$trigger-id',
        label: 'Back to trigger',
        params: { 'trigger-id': triggerId },
      }}
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : trigger ? (
        <TaskTriggerForm trigger={trigger} onSuccess={goBack} onCancel={goBack} />
      ) : (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Trigger not found</p>
          <p className="text-sm">This trigger may have been deleted.</p>
        </div>
      )}
    </AdminPage>
  )
}

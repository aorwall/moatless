import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { TaskTriggerForm } from '@/features/integrations/components/task-trigger-form'
import { useTrigger } from '@/features/integrations/hooks/use-integrations'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'

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

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!trigger) {
    return (
      <DetailNotFound title="Trigger not found" description="This trigger may have been deleted." />
    )
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">Edit Trigger</h1>
        <p className="text-sm text-muted-foreground">Update trigger source and rule fields.</p>
      </div>
      <TaskTriggerForm trigger={trigger} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}

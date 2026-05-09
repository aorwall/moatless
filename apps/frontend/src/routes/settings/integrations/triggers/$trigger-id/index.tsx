import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { SettingsPage } from '@/features/settings/components/settings-page'
import { TaskTriggerForm } from '@/features/settings/components/task-trigger-form'
import { useTaskTriggers } from '@/features/settings/hooks/use-integrations'

export const Route = createFileRoute('/settings/integrations/triggers/$trigger-id/')({
  component: EditTriggerPage,
})

function EditTriggerPage() {
  const params = Route.useParams()
  const triggerId = params['trigger-id']
  const navigate = useNavigate()
  const { data: triggers, isLoading } = useTaskTriggers()
  const trigger = triggers?.find((item) => item.id === triggerId)

  const goBack = () => navigate({ to: '/settings/integrations/triggers' })

  return (
    <SettingsPage
      title="Edit Trigger"
      description="Update trigger routing."
      backTo={{ to: '/settings/integrations/triggers', label: 'Back to triggers' }}
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : !trigger ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Trigger not found</p>
          <p className="text-sm">This trigger may have been deleted.</p>
        </div>
      ) : (
        <TaskTriggerForm trigger={trigger} onSuccess={goBack} onCancel={goBack} />
      )}
    </SettingsPage>
  )
}

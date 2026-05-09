import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Loader2 } from 'lucide-react'
import { TaskTriggerForm } from '@/features/settings/components/task-trigger-form'
import { useTaskTriggers } from '@/features/settings/hooks/use-integrations'
import { Button } from '@/lib/components/ui/button'

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
    <div className="container mx-auto max-w-3xl px-6 py-6 space-y-6">
      <Button variant="ghost" size="sm" asChild>
        <Link to="/settings/integrations/triggers">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back to triggers
        </Link>
      </Button>

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
        <>
          <div>
            <h1 className="text-2xl font-semibold">Edit Trigger</h1>
            <p className="text-sm text-muted-foreground">Update trigger routing.</p>
          </div>
          <TaskTriggerForm trigger={trigger} onSuccess={goBack} onCancel={goBack} />
        </>
      )}
    </div>
  )
}

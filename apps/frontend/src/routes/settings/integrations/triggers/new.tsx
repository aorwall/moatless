import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import { TaskTriggerForm } from '@/features/settings/components/task-trigger-form'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/settings/integrations/triggers/new')({
  component: NewTriggerPage,
})

function NewTriggerPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/settings/integrations/triggers' })

  return (
    <div className="container mx-auto max-w-3xl px-6 py-6 space-y-6">
      <Button variant="ghost" size="sm" asChild>
        <Link to="/settings/integrations/triggers">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back to triggers
        </Link>
      </Button>

      <div>
        <h1 className="text-2xl font-semibold">New Trigger</h1>
        <p className="text-sm text-muted-foreground">Create an integration trigger.</p>
      </div>

      <TaskTriggerForm trigger={null} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}

import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import { TaskRouteForm } from '@/features/settings/components/task-route-form'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/settings/integrations/routes/new')({
  component: NewRoutePage,
})

function NewRoutePage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/settings/integrations/routes' })

  return (
    <div className="container mx-auto max-w-3xl px-6 py-6 space-y-6">
      <Button variant="ghost" size="sm" asChild>
        <Link to="/settings/integrations/routes">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back to routes
        </Link>
      </Button>

      <div>
        <h1 className="text-2xl font-semibold">New Route</h1>
        <p className="text-sm text-muted-foreground">Create a reusable route for triggers.</p>
      </div>

      <TaskRouteForm route={null} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}

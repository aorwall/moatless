import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Loader2 } from 'lucide-react'
import { TaskRouteForm } from '@/features/settings/components/task-route-form'
import { useTaskRoutes } from '@/features/settings/hooks/use-integrations'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/settings/integrations/routes/$route-id/')({
  component: EditRoutePage,
})

function EditRoutePage() {
  const params = Route.useParams()
  const routeId = params['route-id']
  const navigate = useNavigate()
  const { data: routes, isLoading } = useTaskRoutes()
  const route = routes?.find((item) => item.id === routeId)

  const goBack = () => navigate({ to: '/settings/integrations/routes' })

  return (
    <div className="container mx-auto max-w-3xl px-6 py-6 space-y-6">
      <Button variant="ghost" size="sm" asChild>
        <Link to="/settings/integrations/routes">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back to routes
        </Link>
      </Button>

      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : !route ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Route not found</p>
          <p className="text-sm">This route may have been deleted.</p>
        </div>
      ) : (
        <>
          <div>
            <h1 className="text-2xl font-semibold">Edit Route</h1>
            <p className="text-sm text-muted-foreground">Update task route defaults.</p>
          </div>
          <TaskRouteForm route={route} onSuccess={goBack} onCancel={goBack} />
        </>
      )}
    </div>
  )
}

import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { SettingsPage } from '@/features/settings/components/settings-page'
import { TaskRouteForm } from '@/features/settings/components/task-route-form'
import { useTaskRoutes } from '@/features/settings/hooks/use-integrations'

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
    <SettingsPage
      title="Edit Route"
      description="Update task route defaults."
      backTo={{ to: '/settings/integrations/routes', label: 'Back to routes' }}
    >
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
        <TaskRouteForm route={route} onSuccess={goBack} onCancel={goBack} />
      )}
    </SettingsPage>
  )
}

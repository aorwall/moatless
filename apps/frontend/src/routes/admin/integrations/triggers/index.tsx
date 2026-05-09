import type {
  ProviderConnectionResponse,
  RepositoryResponse,
  TaskRouteResponse,
  TaskTriggerResponse,
} from '@moatless/api-client'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { Bell, Loader2, Plus, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useDeleteTaskTrigger,
  useProviderConnections,
  useTaskRoutes,
  useTaskTriggers,
  useUpdateTaskTrigger,
} from '@/features/integrations/hooks/use-integrations'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Switch } from '@/lib/components/ui/switch'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { getErrorMessage } from '@/lib/utils/error'

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
}

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
}

function getRouteLabel(route: TaskRouteResponse, repositoriesById: Map<string, string>): string {
  const repositoryName = repositoriesById.get(route.repositoryId) ?? route.repositoryId
  const taskName = route.taskName ? ` - ${route.taskName}` : ''
  return `${repositoryName} - ${getRoutingModeLabel(route.routingMode)}${taskName}`
}

function getConnectionLabel(connection: ProviderConnectionResponse | undefined): string {
  return connection?.externalAccountId ?? 'Missing connection'
}

function getSourceLabel(trigger: TaskTriggerResponse): string {
  return trigger.sourceName || trigger.sourceMatcher
}

function createRepositoryMap(repositories: RepositoryResponse[] | undefined) {
  return new Map((repositories ?? []).map((repository) => [repository.id, repository.name]))
}

export const Route = createFileRoute('/admin/integrations/triggers/')({
  component: TriggersListPage,
})

function TriggersListPage() {
  const navigate = useNavigate()
  const { data: triggers, isLoading: triggersLoading, error: triggersError } = useTaskTriggers()
  const { data: routes, isLoading: routesLoading } = useTaskRoutes()
  const { data: connections, isLoading: connectionsLoading } = useProviderConnections()
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList()

  const updateTrigger = useUpdateTaskTrigger()
  const deleteTrigger = useDeleteTaskTrigger()
  const [triggerToDelete, setTriggerToDelete] = useState<TaskTriggerResponse | null>(null)

  const triggerItems = triggers ?? []
  const routeItems = routes ?? []
  const connectionItems = connections ?? []

  const repositoriesById = useMemo(() => createRepositoryMap(repositories), [repositories])
  const routesById = useMemo(
    () => new Map(routeItems.map((route) => [route.id, route])),
    [routeItems],
  )
  const connectionsById = useMemo(
    () => new Map(connectionItems.map((connection) => [connection.id, connection])),
    [connectionItems],
  )

  const isLoading = triggersLoading || routesLoading || connectionsLoading || repositoriesLoading
  const hasRoutes = routeItems.length > 0

  const handleToggleActive = (trigger: TaskTriggerResponse) => {
    updateTrigger.mutate(
      { id: trigger.id, request: { active: !trigger.active } },
      {
        onError: (mutationError) => {
          toast.error('Failed to update trigger', {
            description: getErrorMessage(mutationError),
          })
        },
      },
    )
  }

  const confirmDeleteTrigger = () => {
    if (!triggerToDelete) {
      return
    }
    deleteTrigger.mutate(triggerToDelete.id, {
      onSuccess: () => {
        toast.success('Trigger deleted')
        setTriggerToDelete(null)
      },
      onError: (mutationError) => {
        toast.error('Failed to delete trigger', {
          description: getErrorMessage(mutationError),
        })
      },
    })
  }

  return (
    <AdminPage
      title="Triggers"
      description="Inbound integration triggers that route messages to tasks."
      actions={
        <Button
          onClick={() => navigate({ to: '/admin/integrations/triggers/new' })}
          size="sm"
          disabled={!hasRoutes}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Trigger
        </Button>
      }
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : triggersError ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>Failed to load triggers. Please try again later.</AlertDescription>
        </Alert>
      ) : triggerItems.length === 0 ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <Bell className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No triggers yet</p>
          {!hasRoutes && (
            <p className="text-sm">
              Create a{' '}
              <Link
                to="/admin/integrations/routes"
                className="text-primary underline hover:no-underline"
              >
                task route
              </Link>{' '}
              before adding a trigger.
            </p>
          )}
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Provider</TableHead>
              <TableHead>Source</TableHead>
              <TableHead>Connection</TableHead>
              <TableHead>Task Route</TableHead>
              <TableHead>Active</TableHead>
              <TableHead className="w-20" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {triggerItems.map((trigger) => {
              const route = routesById.get(trigger.routeId)
              const connection = connectionsById.get(trigger.connectionId)

              return (
                <TableRow
                  key={trigger.id}
                  className="cursor-pointer hover:bg-muted/50"
                  onClick={() =>
                    navigate({
                      to: '/admin/integrations/triggers/$trigger-id',
                      params: { 'trigger-id': trigger.id },
                    })
                  }
                >
                  <TableCell>
                    <Badge variant="secondary">{getProviderLabel(trigger.provider)}</Badge>
                  </TableCell>
                  <TableCell>
                    <div className="font-medium">{getSourceLabel(trigger)}</div>
                    {trigger.sourceName && (
                      <div className="text-xs text-muted-foreground">{trigger.sourceMatcher}</div>
                    )}
                  </TableCell>
                  <TableCell>{getConnectionLabel(connection)}</TableCell>
                  <TableCell>
                    {route ? getRouteLabel(route, repositoriesById) : trigger.routeId}
                  </TableCell>
                  <TableCell onClick={(e) => e.stopPropagation()}>
                    <Switch
                      checked={trigger.active}
                      onCheckedChange={() => handleToggleActive(trigger)}
                      disabled={updateTrigger.isPending}
                      aria-label={`Toggle ${getSourceLabel(trigger)} active`}
                    />
                  </TableCell>
                  <TableCell onClick={(e) => e.stopPropagation()}>
                    <div className="flex items-center gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => setTriggerToDelete(trigger)}
                        title="Delete"
                        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}

      <AlertDialog open={!!triggerToDelete} onOpenChange={() => setTriggerToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Trigger</AlertDialogTitle>
            <AlertDialogDescription>
              Delete trigger "{triggerToDelete ? getSourceLabel(triggerToDelete) : ''}"?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmDeleteTrigger}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteTrigger.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Trash2 className="mr-2 h-4 w-4" />
              )}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </AdminPage>
  )
}

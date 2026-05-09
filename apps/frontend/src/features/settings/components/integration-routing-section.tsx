import type {
  ProviderConnectionResponse,
  RepositoryResponse,
  TaskRouteResponse,
  TaskTriggerResponse,
} from '@moatless/api-client'
import { Bell, Loader2, Pencil, Plus, Route, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { TaskRouteFormDialog } from '@/features/settings/components/task-route-form-dialog'
import { TaskTriggerFormDialog } from '@/features/settings/components/task-trigger-form-dialog'
import {
  useDeleteTaskTrigger,
  useIntegrationProviders,
  useProviderConnections,
  useTaskRoutes,
  useTaskTriggers,
  useUpdateTaskTrigger,
} from '@/features/settings/hooks/use-integrations'
import { useTagsList } from '@/features/tags'
import { ConfigurationGroup, ConfigurationSectionWrapper } from '@/lib/components/configuration'
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { getErrorMessage } from '@/lib/utils/error'

const ROUTING_MODE_LABELS: Record<string, string> = {
  thread_based: 'Thread Based',
  per_message: 'Per Message',
  ongoing: 'Ongoing',
}

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
}

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  return date.toLocaleString()
}

function createRepositoryMap(repositories: RepositoryResponse[] | undefined) {
  return new Map((repositories ?? []).map((repository) => [repository.id, repository.name]))
}

function getRepositoryName(repositoryId: string, repositoriesById: Map<string, string>): string {
  return repositoriesById.get(repositoryId) ?? repositoryId
}

function getRouteLabel(route: TaskRouteResponse, repositoriesById: Map<string, string>): string {
  const repositoryName = getRepositoryName(route.repositoryId, repositoriesById)
  const taskName = route.taskName ? ` - ${route.taskName}` : ''
  return `${repositoryName} - ${getRoutingModeLabel(route.routingMode)}${taskName}`
}

function getConnectionLabel(connection: ProviderConnectionResponse | undefined): string {
  return connection?.externalAccountId ?? 'Missing connection'
}

function getSourceLabel(trigger: TaskTriggerResponse): string {
  return trigger.sourceName || trigger.sourceMatcher
}

interface TaskRoutesTabProps {
  routes: TaskRouteResponse[]
  repositories: RepositoryResponse[]
  repositoriesById: Map<string, string>
  onAdd: () => void
  onEdit: (route: TaskRouteResponse) => void
}

function TaskRoutesTab({
  routes,
  repositories,
  repositoriesById,
  onAdd,
  onEdit,
}: TaskRoutesTabProps) {
  return (
    <div className="space-y-4">
      <div className="flex justify-end">
        <Button onClick={onAdd} size="sm" disabled={repositories.length === 0}>
          <Plus className="mr-2 h-4 w-4" />
          Add Route
        </Button>
      </div>

      {routes.length === 0 ? (
        <div className="rounded-md border py-8 text-center text-muted-foreground">
          <Route className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No task routes yet</p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Repository</TableHead>
              <TableHead>Routing Mode</TableHead>
              <TableHead>Task Name</TableHead>
              <TableHead>Tags</TableHead>
              <TableHead>Prompt Template</TableHead>
              <TableHead>Created</TableHead>
              <TableHead>Updated</TableHead>
              <TableHead className="w-12" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {routes.map((route) => (
              <TableRow key={route.id}>
                <TableCell className="font-medium">
                  {getRepositoryName(route.repositoryId, repositoriesById)}
                </TableCell>
                <TableCell>{getRoutingModeLabel(route.routingMode)}</TableCell>
                <TableCell>{route.taskName || '-'}</TableCell>
                <TableCell>{route.tagIds.length}</TableCell>
                <TableCell>
                  <Badge variant={route.promptTemplate ? 'secondary' : 'outline'}>
                    {route.promptTemplate ? 'Configured' : 'None'}
                  </Badge>
                </TableCell>
                <TableCell>{formatTimestamp(route.createdAt)}</TableCell>
                <TableCell>{formatTimestamp(route.updatedAt)}</TableCell>
                <TableCell>
                  <Button variant="ghost" size="icon" onClick={() => onEdit(route)} title="Edit">
                    <Pencil className="h-4 w-4" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}

interface TriggersTabProps {
  triggers: TaskTriggerResponse[]
  routesById: Map<string, TaskRouteResponse>
  connectionsById: Map<string, ProviderConnectionResponse>
  repositoriesById: Map<string, string>
  hasRoutes: boolean
  onAdd: () => void
  onEdit: (trigger: TaskTriggerResponse) => void
  onDelete: (trigger: TaskTriggerResponse) => void
  onToggleActive: (trigger: TaskTriggerResponse) => void
  isUpdatingActive: boolean
}

function TriggersTab({
  triggers,
  routesById,
  connectionsById,
  repositoriesById,
  hasRoutes,
  onAdd,
  onEdit,
  onDelete,
  onToggleActive,
  isUpdatingActive,
}: TriggersTabProps) {
  return (
    <div className="space-y-4">
      <div className="flex justify-end">
        <Button onClick={onAdd} size="sm" disabled={!hasRoutes}>
          <Plus className="mr-2 h-4 w-4" />
          Add Trigger
        </Button>
      </div>

      {triggers.length === 0 ? (
        <div className="rounded-md border py-8 text-center text-muted-foreground">
          <Bell className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No triggers yet</p>
          {!hasRoutes && <p className="text-sm">Create a task route before adding a trigger.</p>}
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
            {triggers.map((trigger) => {
              const route = routesById.get(trigger.routeId)
              const connection = connectionsById.get(trigger.connectionId)

              return (
                <TableRow key={trigger.id}>
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
                  <TableCell>
                    <Switch
                      checked={trigger.active}
                      onCheckedChange={() => onToggleActive(trigger)}
                      disabled={isUpdatingActive}
                      aria-label={`Toggle ${getSourceLabel(trigger)} active`}
                    />
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => onEdit(trigger)}
                        title="Edit"
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => onDelete(trigger)}
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
    </div>
  )
}

export function IntegrationRoutingSection() {
  const {
    data: providers,
    isLoading: providersLoading,
    error: providersError,
  } = useIntegrationProviders()
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList()
  const { data: tags } = useTagsList()
  const { data: routes, isLoading: routesLoading, error: routesError } = useTaskRoutes()
  const { data: triggers, isLoading: triggersLoading, error: triggersError } = useTaskTriggers()
  const {
    data: connections,
    isLoading: connectionsLoading,
    error: connectionsError,
  } = useProviderConnections()
  const updateTrigger = useUpdateTaskTrigger()
  const deleteTrigger = useDeleteTaskTrigger()

  const [routeDialogOpen, setRouteDialogOpen] = useState(false)
  const [triggerDialogOpen, setTriggerDialogOpen] = useState(false)
  const [editingRoute, setEditingRoute] = useState<TaskRouteResponse | null>(null)
  const [editingTrigger, setEditingTrigger] = useState<TaskTriggerResponse | null>(null)
  const [triggerToDelete, setTriggerToDelete] = useState<TaskTriggerResponse | null>(null)

  const routeItems = routes ?? []
  const triggerItems = triggers ?? []
  const providerItems = providers ?? []
  const repositoryItems = repositories ?? []
  const tagItems = tags ?? []
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
  const isLoading =
    providersLoading ||
    repositoriesLoading ||
    routesLoading ||
    triggersLoading ||
    connectionsLoading
  const error = providersError || routesError || triggersError || connectionsError

  const handleAddRoute = () => {
    setEditingRoute(null)
    setRouteDialogOpen(true)
  }

  const handleAddTrigger = () => {
    setEditingTrigger(null)
    setTriggerDialogOpen(true)
  }

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

  const routeLabelForId = (routeId: string): string => {
    const route = routesById.get(routeId)
    return route ? getRouteLabel(route, repositoriesById) : routeId
  }

  if (isLoading) {
    return (
      <div className="flex h-32 items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load integrations. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Integrations" showSeparator={false}>
        <Tabs defaultValue="task-routes" className="gap-4">
          <TabsList>
            <TabsTrigger value="task-routes">Task Routes</TabsTrigger>
            <TabsTrigger value="triggers">Triggers</TabsTrigger>
          </TabsList>

          <TabsContent value="task-routes">
            <TaskRoutesTab
              routes={routeItems}
              repositories={repositoryItems}
              repositoriesById={repositoriesById}
              onAdd={handleAddRoute}
              onEdit={(route) => {
                setEditingRoute(route)
                setRouteDialogOpen(true)
              }}
            />
          </TabsContent>

          <TabsContent value="triggers">
            <TriggersTab
              triggers={triggerItems}
              routesById={routesById}
              connectionsById={connectionsById}
              repositoriesById={repositoriesById}
              hasRoutes={routeItems.length > 0}
              onAdd={handleAddTrigger}
              onEdit={(trigger) => {
                setEditingTrigger(trigger)
                setTriggerDialogOpen(true)
              }}
              onDelete={setTriggerToDelete}
              onToggleActive={handleToggleActive}
              isUpdatingActive={updateTrigger.isPending}
            />
          </TabsContent>
        </Tabs>
      </ConfigurationGroup>

      <TaskRouteFormDialog
        open={routeDialogOpen}
        onOpenChange={setRouteDialogOpen}
        route={editingRoute}
        repositories={repositoryItems}
        tags={tagItems}
      />

      <TaskTriggerFormDialog
        open={triggerDialogOpen}
        onOpenChange={setTriggerDialogOpen}
        trigger={editingTrigger}
        providers={providerItems}
        routes={routeItems}
        connections={connectionItems}
        getRouteLabel={routeLabelForId}
      />

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
    </ConfigurationSectionWrapper>
  )
}

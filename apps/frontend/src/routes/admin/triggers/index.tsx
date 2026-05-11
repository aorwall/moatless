import type {
  AdapterConnectionResponse,
  AdapterKind,
  RepositoryResponse,
  TriggerResponse,
} from '@moatless/api-client'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Bell, CalendarClock, Loader2, Plus, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useDeleteTrigger,
  useIntegrationConnections,
  useTriggers,
  useUpdateTrigger,
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

type TriggerSource = TriggerResponse['source']

const ADAPTER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

function isAdapterEventSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { adapterKind: AdapterKind }> {
  return 'adapterKind' in source
}

function isScheduleSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { cronExpression: string }> {
  return 'cronExpression' in source
}

function getAdapterLabel(adapter: string): string {
  return ADAPTER_LABELS[adapter] ?? adapter
}

function getSourceLabel(trigger: TriggerResponse): string {
  if (isAdapterEventSource(trigger.source)) {
    return trigger.source.sourceName || trigger.source.sourceMatcher
  }
  if (isScheduleSource(trigger.source)) {
    return trigger.name || trigger.source.cronExpression
  }
  return trigger.name || trigger.id
}

function getRepositoryName(
  repositoriesById: Map<string, RepositoryResponse>,
  repositoryId: string,
): string {
  return repositoriesById.get(repositoryId)?.name ?? repositoryId
}

function getConnectionLabel(
  connectionsById: Map<string, AdapterConnectionResponse>,
  connectionId: string,
): string {
  return connectionsById.get(connectionId)?.externalAccountId ?? 'Missing connection'
}

export const Route = createFileRoute('/admin/triggers/')({
  component: TriggersListPage,
})

function TriggersListPage() {
  const navigate = useNavigate()
  const { data: triggers, isLoading: triggersLoading, error: triggersError } = useTriggers()
  const { data: connections, isLoading: connectionsLoading } = useIntegrationConnections()
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList()
  const updateTrigger = useUpdateTrigger()
  const deleteTrigger = useDeleteTrigger()
  const [triggerToDelete, setTriggerToDelete] = useState<TriggerResponse | null>(null)

  const triggerItems = triggers ?? []
  const connectionsById = useMemo(
    () => new Map((connections ?? []).map((connection) => [connection.id, connection])),
    [connections],
  )
  const repositoriesById = useMemo(
    () => new Map((repositories ?? []).map((repository) => [repository.id, repository])),
    [repositories],
  )
  const isLoading = triggersLoading || connectionsLoading || repositoriesLoading

  const handleToggleActive = (trigger: TriggerResponse) => {
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
      description="Adapter events and schedules that start task work."
      actions={
        <Button onClick={() => navigate({ to: '/admin/triggers/new' })} size="sm">
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
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Kind</TableHead>
              <TableHead>Source</TableHead>
              <TableHead>Rule</TableHead>
              <TableHead>Active</TableHead>
              <TableHead className="w-20" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {triggerItems.map((trigger) => {
              const source = trigger.source
              const isAdapterEvent = isAdapterEventSource(source)

              return (
                <TableRow
                  key={trigger.id}
                  className="cursor-pointer hover:bg-muted/50"
                  onClick={() =>
                    navigate({
                      to: '/admin/triggers/$trigger-id',
                      params: { 'trigger-id': trigger.id },
                    })
                  }
                >
                  <TableCell>
                    <Badge variant="secondary">
                      {isAdapterEvent ? getAdapterLabel(source.adapterKind) : 'Schedule'}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <div className="font-medium">{getSourceLabel(trigger)}</div>
                    {isAdapterEvent ? (
                      <div className="text-xs text-muted-foreground">
                        {getConnectionLabel(connectionsById, source.connectionId)}
                      </div>
                    ) : isScheduleSource(source) ? (
                      <div className="text-xs text-muted-foreground">{source.cronExpression}</div>
                    ) : null}
                  </TableCell>
                  <TableCell>
                    <div>{getRepositoryName(repositoriesById, trigger.rule.repositoryId)}</div>
                    <div className="text-xs text-muted-foreground">{trigger.rule.routingMode}</div>
                  </TableCell>
                  <TableCell onClick={(event) => event.stopPropagation()}>
                    <Switch
                      checked={trigger.active}
                      onCheckedChange={() => handleToggleActive(trigger)}
                      disabled={updateTrigger.isPending}
                      aria-label={`Toggle ${getSourceLabel(trigger)} active`}
                    />
                  </TableCell>
                  <TableCell onClick={(event) => event.stopPropagation()}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setTriggerToDelete(trigger)}
                      title="Delete"
                      className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
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
                <CalendarClock className="mr-2 h-4 w-4" />
              )}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </AdminPage>
  )
}

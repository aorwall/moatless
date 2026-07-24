import type { AdapterConnectionResponse, Loop, LoopRecentTaskItem } from '@moatless/api-client'
import { createLazyFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { Loader2, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useDeleteTrigger,
  useIntegrationConnections,
  useTrigger,
  useTriggerRecentTasks,
  useUpdateTrigger,
} from '@/features/integrations/hooks/use-integrations'
import {
  getAdapterLabel,
  getSourceLabel,
  isAdapterEventSource,
  isScheduleSource,
} from '@/features/integrations/utils/trigger-display'
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
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailItem } from '@/lib/components/ui/detail-item'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
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

export const Route = createLazyFileRoute('/admin/triggers/$trigger-id/')({
  component: TriggerDetailPage,
})

function SourceInfoSection({
  trigger,
  connection,
}: {
  trigger: Loop
  connection: AdapterConnectionResponse | undefined
}) {
  const source = trigger.source

  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Source</h2>
      {isAdapterEventSource(source) ? (
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Adapter" value={getAdapterLabel(source.adapterKind)} />
          <DetailItem label="Source matcher" value={source.sourceMatcher} />
          <DetailItem label="Source name" value={source.sourceName ?? '-'} />
          <DetailItem
            label="Connection"
            value={connection?.externalAccountId ?? source.connectionId}
          />
          <DetailItem label="Allow direct send" value={source.allowDirectSend ? 'Yes' : 'No'} />
          <DetailItem label="Close task on close" value={source.closeTaskOnClose ? 'Yes' : 'No'} />
          <DetailItem label="Notify on success" value={source.notifyOnSuccess ? 'Yes' : 'No'} />
        </dl>
      ) : isScheduleSource(source) ? (
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Cron expression" value={source.cronExpression} />
          <DetailItem label="Timezone" value={source.timezone} />
          <DetailItem label="Next fire" value={formatTimestamp(source.nextFireAt)} />
          <DetailItem label="Last fired" value={formatTimestamp(source.lastFiredAt)} />
          <div className="sm:col-span-2">
            <dt className="text-xs font-medium text-muted-foreground">Message template</dt>
            <dd className="mt-1 whitespace-pre-wrap text-sm">{source.messageTemplate}</dd>
          </div>
        </dl>
      ) : (
        <p className="text-sm text-muted-foreground">Manual trigger</p>
      )}
    </div>
  )
}

function RuleInfoSection({ trigger }: { trigger: Loop }) {
  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Rule</h2>
      <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <DetailItem label="Repository ID" value={trigger.rule.repositoryId} />
        <DetailItem label="Routing mode" value={trigger.rule.routingMode} />
        <DetailItem label="Ongoing task ID" value={trigger.rule.ongoingTaskId ?? '-'} />
        <DetailItem label="Task name" value={trigger.rule.taskName ?? '-'} />
        <DetailItem label="Run as user ID" value={trigger.rule.runAsUserId ?? '-'} />
        <DetailItem label="Agent type" value={trigger.rule.agentType} />
        <DetailItem label="Tags count" value={String(trigger.rule.tagIds.length)} />
        <div className="sm:col-span-2">
          <dt className="text-xs font-medium text-muted-foreground">Prompt</dt>
          <dd className="mt-1 whitespace-pre-wrap text-sm">{trigger.rule.prompt ?? '-'}</dd>
        </div>
      </dl>
    </div>
  )
}

function RecentTasksSection({ items }: { items: LoopRecentTaskItem[] }) {
  if (items.length === 0) {
    return <p className="text-sm text-muted-foreground">No tasks created yet.</p>
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Task</TableHead>
          <TableHead>Status</TableHead>
          <TableHead>Subject</TableHead>
          <TableHead>Created</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {items.map((item) => (
          <TableRow key={item.task.taskId}>
            <TableCell>
              <Link
                to="/tasks/$task-id"
                params={{ 'task-id': item.task.taskId }}
                className="text-primary underline hover:no-underline"
              >
                {item.task.shortId ?? item.task.taskId.slice(0, 8)}
              </Link>
              <div className="text-xs text-muted-foreground">{item.task.title}</div>
            </TableCell>
            <TableCell>
              <Badge variant="secondary">{item.task.status}</Badge>
            </TableCell>
            <TableCell className="text-sm">{item.binding?.subjectKey ?? '-'}</TableCell>
            <TableCell className="text-sm">{formatTimestamp(item.task.createdAt)}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

function TriggerDetailPage() {
  const params = Route.useParams()
  const triggerId = params['trigger-id']
  const navigate = useNavigate()
  const { data: trigger, isLoading: triggerLoading } = useTrigger(triggerId)
  const { data: connections } = useIntegrationConnections()
  const { data: recentTasks, isLoading: tasksLoading } = useTriggerRecentTasks(triggerId, {
    limit: 10,
  })
  const updateTrigger = useUpdateTrigger()
  const deleteTrigger = useDeleteTrigger()
  const [confirmDelete, setConfirmDelete] = useState(false)

  if (triggerLoading) {
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

  const source = trigger.source
  const connection = isAdapterEventSource(source)
    ? connections?.find((item) => item.id === source.connectionId)
    : undefined

  const handleToggleActive = () => {
    updateTrigger.mutate(
      { id: trigger.id, request: { active: !trigger.active } },
      {
        onError: (mutationError) =>
          toast.error('Failed to update trigger', {
            description: getErrorMessage(mutationError),
          }),
      },
    )
  }

  const handleDelete = () => {
    deleteTrigger.mutate(trigger.id, {
      onSuccess: async () => {
        toast.success('Trigger deleted')
        setConfirmDelete(false)
        await navigate({ to: '/admin/triggers' })
      },
      onError: (mutationError) => {
        toast.error('Failed to delete trigger', { description: getErrorMessage(mutationError) })
      },
    })
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        title={getSourceLabel(trigger)}
        subtitle={`${trigger.kind} trigger`}
        badges={
          <Badge variant={trigger.active ? 'default' : 'secondary'}>
            {trigger.active ? 'Active' : 'Inactive'}
          </Badge>
        }
        actions={
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-2">
              <Switch
                checked={trigger.active}
                onCheckedChange={handleToggleActive}
                disabled={updateTrigger.isPending}
                aria-label="Toggle active"
              />
              <span className="text-sm text-muted-foreground">Active</span>
            </div>
            <Button asChild variant="outline">
              <Link to="/admin/triggers/$trigger-id/edit" params={{ 'trigger-id': trigger.id }}>
                Edit
              </Link>
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setConfirmDelete(true)}
              title="Delete"
              className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        }
      />

      <div className="rounded-md border p-4">
        <h2 className="mb-4 text-sm font-semibold">Trigger</h2>
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Kind" value={trigger.kind} />
          <DetailItem label="Name" value={trigger.name ?? '-'} />
          <DetailItem label="Scope" value={trigger.scope} />
          <DetailItem label="Created" value={formatTimestamp(trigger.createdAt)} />
        </dl>
      </div>
      <SourceInfoSection trigger={trigger} connection={connection} />
      <RuleInfoSection trigger={trigger} />
      <div className="rounded-md border p-4">
        <h2 className="mb-4 text-sm font-semibold">Recent Tasks</h2>
        {tasksLoading ? (
          <div className="flex h-16 items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : (
          <RecentTasksSection items={recentTasks ?? []} />
        )}
      </div>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Trigger</AlertDialogTitle>
            <AlertDialogDescription>
              Delete trigger "{getSourceLabel(trigger)}"? This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteTrigger.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

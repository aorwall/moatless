import type { AdapterConnectionResponse, Loop, LoopRecentTaskItem } from '@moatless/api-client'
import { createLazyFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { Loader2, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useIntegrationConnections } from '@/features/integrations/hooks/use-integrations'
import {
  useDeleteLoop,
  useLoop,
  useLoopRecentTasks,
  useUpdateLoop,
} from '@/features/loops/hooks/use-loops'
import {
  getAdapterLabel,
  getSourceLabel,
  isAdapterEventSource,
  isScheduleSource,
} from '@/features/loops/utils/loop-display'
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

export const Route = createLazyFileRoute('/admin/loops/$loop-id/')({
  component: LoopDetailPage,
})

function SourceInfoSection({
  loop,
  connection,
}: {
  loop: Loop
  connection: AdapterConnectionResponse | undefined
}) {
  const source = loop.source

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
        <p className="text-sm text-muted-foreground">Manual loop</p>
      )}
    </div>
  )
}

function RuleInfoSection({ loop }: { loop: Loop }) {
  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Rule</h2>
      <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <DetailItem label="Repository ID" value={loop.rule.repositoryId} />
        <DetailItem label="Routing mode" value={loop.rule.routingMode} />
        <DetailItem label="Ongoing task ID" value={loop.rule.ongoingTaskId ?? '-'} />
        <DetailItem label="Task name" value={loop.rule.taskName ?? '-'} />
        <DetailItem label="Run as user ID" value={loop.rule.runAsUserId ?? '-'} />
        <DetailItem label="Agent type" value={loop.rule.agentType} />
        <DetailItem label="Tags count" value={String(loop.rule.tagIds.length)} />
        <div className="sm:col-span-2">
          <dt className="text-xs font-medium text-muted-foreground">Prompt</dt>
          <dd className="mt-1 whitespace-pre-wrap text-sm">{loop.rule.prompt ?? '-'}</dd>
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

function LoopDetailPage() {
  const params = Route.useParams()
  const loopId = params['loop-id']
  const navigate = useNavigate()
  const { data: loop, isLoading: loopLoading } = useLoop(loopId)
  const { data: connections } = useIntegrationConnections()
  const { data: recentTasks, isLoading: tasksLoading } = useLoopRecentTasks(loopId, {
    limit: 10,
  })
  const updateLoop = useUpdateLoop()
  const deleteLoop = useDeleteLoop()
  const [confirmDelete, setConfirmDelete] = useState(false)

  if (loopLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!loop) {
    return <DetailNotFound title="Loop not found" description="This loop may have been deleted." />
  }

  const source = loop.source
  const connection = isAdapterEventSource(source)
    ? connections?.find((item) => item.id === source.connectionId)
    : undefined

  const handleToggleActive = () => {
    updateLoop.mutate(
      { id: loop.id, request: { active: !loop.active } },
      {
        onError: (mutationError) =>
          toast.error('Failed to update loop', {
            description: getErrorMessage(mutationError),
          }),
      },
    )
  }

  const handleDelete = () => {
    deleteLoop.mutate(loop.id, {
      onSuccess: async () => {
        toast.success('Loop deleted')
        setConfirmDelete(false)
        await navigate({ to: '/admin/loops' })
      },
      onError: (mutationError) => {
        toast.error('Failed to delete loop', { description: getErrorMessage(mutationError) })
      },
    })
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        title={getSourceLabel(loop)}
        subtitle={`${loop.kind} loop`}
        badges={
          <Badge variant={loop.active ? 'default' : 'secondary'}>
            {loop.active ? 'Active' : 'Inactive'}
          </Badge>
        }
        actions={
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-2">
              <Switch
                checked={loop.active}
                onCheckedChange={handleToggleActive}
                disabled={updateLoop.isPending}
                aria-label="Toggle active"
              />
              <span className="text-sm text-muted-foreground">Active</span>
            </div>
            <Button asChild variant="outline">
              <Link to="/admin/loops/$loop-id/edit" params={{ 'loop-id': loop.id }}>
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
        <h2 className="mb-4 text-sm font-semibold">Loop</h2>
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Kind" value={loop.kind} />
          <DetailItem label="Name" value={loop.name ?? '-'} />
          <DetailItem label="Scope" value={loop.scope} />
          <DetailItem label="Created" value={formatTimestamp(loop.createdAt)} />
        </dl>
      </div>
      <SourceInfoSection loop={loop} connection={connection} />
      <RuleInfoSection loop={loop} />
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
            <AlertDialogTitle>Delete Loop</AlertDialogTitle>
            <AlertDialogDescription>
              Delete loop "{getSourceLabel(loop)}"? This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteLoop.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

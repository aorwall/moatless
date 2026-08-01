import {
  type AdapterConnectionResponse,
  type Loop,
  LoopExecutionState,
  type LoopRecentTaskItem,
} from '@moatless/api-client'
import { createLazyFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { Loader2, Pause, Play, RotateCcw, Trash2, Unplug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useAdminUsersList } from '@/features/admin/hooks/use-admin-users-teams'
import { useIntegrationConnections } from '@/features/integrations/hooks/use-integrations'
import {
  useActivateLoop,
  useDeleteLoop,
  useLoop,
  useLoopRecentTasks,
  useOverrideLoop,
  usePauseLoop,
  useResetGitLoop,
  useResumeLoop,
} from '@/features/loops/hooks/use-loops'
import {
  getAdapterLabel,
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
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

function ConfigInfoSection({ loop }: { loop: Loop }) {
  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Config</h2>
      <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <DetailItem label="Repository ID" value={loop.config.repositoryId} />
        <DetailItem label="Routing mode" value={loop.config.routingMode} />
        <DetailItem label="Ongoing task ID" value={loop.config.ongoingTaskId ?? '-'} />
        <DetailItem label="Task name" value={loop.config.taskName ?? '-'} />
        <DetailItem label="Run as user ID" value={loop.config.runAsUserId ?? '-'} />
        <DetailItem label="Agent type" value={loop.config.agentType} />
        <DetailItem label="Tags count" value={String(loop.config.tagIds.length)} />
        <div className="sm:col-span-2">
          <dt className="text-xs font-medium text-muted-foreground">Prompt</dt>
          <dd className="mt-1 whitespace-pre-wrap text-sm">{loop.config.prompt ?? '-'}</dd>
        </div>
      </dl>
    </div>
  )
}

function executionLabel(state: Loop['executionState']): string {
  switch (state) {
    case LoopExecutionState.active:
      return 'Active'
    case LoopExecutionState.paused:
      return 'Paused'
    case LoopExecutionState.awaiting_approval:
      return 'Awaiting approval'
  }
}

function ExecutionPanel({ loop }: { loop: Loop }) {
  const usersQuery = useAdminUsersList()
  const activateLoop = useActivateLoop()
  const pauseLoop = usePauseLoop()
  const resumeLoop = useResumeLoop()
  const [runAsUserId, setRunAsUserId] = useState(loop.config.runAsUserId ?? '')
  const isPending = activateLoop.isPending || pauseLoop.isPending || resumeLoop.isPending
  const selectedUser = usersQuery.data?.users.find((user) => user.id === loop.config.runAsUserId)

  const activate = () => {
    if (!runAsUserId) {
      toast.error('Select a run-as user')
      return
    }
    activateLoop.mutate(
      { id: loop.id, request: { runAsUserId } },
      {
        onSuccess: () => toast.success('Loop activated'),
        onError: (error) =>
          toast.error('Failed to activate Loop', { description: getErrorMessage(error) }),
      },
    )
  }

  const pause = () => {
    pauseLoop.mutate(loop.id, {
      onSuccess: () => toast.success('Loop paused'),
      onError: (error) =>
        toast.error('Failed to pause Loop', { description: getErrorMessage(error) }),
    })
  }

  const resume = () => {
    resumeLoop.mutate(loop.id, {
      onSuccess: () => toast.success('Loop resumed'),
      onError: (error) =>
        toast.error('Failed to resume Loop', { description: getErrorMessage(error) }),
    })
  }

  return (
    <div className="rounded-md border p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">Execution approval</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Only administrators can choose the identity used by this Loop.
          </p>
        </div>
        <Badge
          variant={loop.executionState === LoopExecutionState.active ? 'default' : 'secondary'}
        >
          {executionLabel(loop.executionState)}
        </Badge>
      </div>

      <div className="mt-4 grid gap-3 sm:grid-cols-[1fr_auto]">
        <Select value={runAsUserId} onValueChange={setRunAsUserId} disabled={isPending}>
          <SelectTrigger aria-label="Run as user">
            <SelectValue placeholder="Select a user" />
          </SelectTrigger>
          <SelectContent>
            {(usersQuery.data?.users ?? []).map((user) => (
              <SelectItem key={user.id} value={user.id}>
                {user.name || user.login} ({user.login})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex gap-2">
          {loop.executionState === LoopExecutionState.active ? (
            <Button variant="outline" onClick={pause} disabled={isPending}>
              <Pause className="mr-2 h-4 w-4" />
              Pause
            </Button>
          ) : null}
          {loop.executionState === LoopExecutionState.paused ? (
            <Button variant="outline" onClick={resume} disabled={isPending}>
              <Play className="mr-2 h-4 w-4" />
              Resume
            </Button>
          ) : null}
          <Button onClick={activate} disabled={isPending || !runAsUserId}>
            {activateLoop.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            {loop.executionState === LoopExecutionState.awaiting_approval
              ? 'Activate'
              : 'Apply user'}
          </Button>
        </div>
      </div>
      {loop.config.runAsUserId ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Approved user: {selectedUser?.name || selectedUser?.login || loop.config.runAsUserId}
        </p>
      ) : null}
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

interface LoopConfigurationActionsProps {
  loop: Loop
  overriding: boolean
  resetting: boolean
  onOverride: () => void
  onResetGit: () => void
}

function LoopConfigurationActions({
  loop,
  overriding,
  resetting,
  onOverride,
  onResetGit,
}: LoopConfigurationActionsProps) {
  return (
    <>
      {loop.syncedFromGit ? (
        <Button variant="outline" onClick={onOverride} disabled={overriding}>
          <Unplug className="mr-2 h-4 w-4" />
          Override
        </Button>
      ) : null}
      {loop.sourceRepositoryId && !loop.syncedFromGit ? (
        <Button variant="outline" onClick={onResetGit} disabled={resetting}>
          <RotateCcw className="mr-2 h-4 w-4" />
          {loop.deleted ? 'Restore from Git' : 'Reset to Git'}
        </Button>
      ) : null}
      {!(loop.syncedFromGit || loop.deleted) ? (
        <Button asChild variant="outline">
          <Link to="/admin/loops/$loop-id/edit" params={{ 'loop-id': loop.id }}>
            Edit
          </Link>
        </Button>
      ) : null}
    </>
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
  const deleteLoop = useDeleteLoop()
  const overrideLoop = useOverrideLoop()
  const resetGitLoop = useResetGitLoop()
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

  const handleOverride = () => {
    overrideLoop.mutate(loop.id, {
      onSuccess: () => toast.success('Loop configuration can now be edited locally'),
      onError: (error) =>
        toast.error('Failed to override Loop', { description: getErrorMessage(error) }),
    })
  }

  const handleResetGit = () => {
    resetGitLoop.mutate(loop.id, {
      onSuccess: (result) => {
        if (result.resynced) {
          toast.success('Loop reset to Git and returned to awaiting approval')
        } else {
          toast.error('Loop was not reset', { description: result.error ?? undefined })
        }
      },
      onError: (error) =>
        toast.error('Failed to reset Loop', { description: getErrorMessage(error) }),
    })
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
        title={loop.name}
        subtitle={`${loop.kind} loop`}
        badges={
          <div className="flex gap-2">
            <Badge variant={loop.active ? 'default' : 'secondary'}>
              {executionLabel(loop.executionState)}
            </Badge>
            <Badge variant="outline">{loop.syncedFromGit ? 'Git' : 'Manual'}</Badge>
            {loop.deleted ? <Badge variant="destructive">Deleted</Badge> : null}
          </div>
        }
        actions={
          <div className="flex items-center gap-2">
            <LoopConfigurationActions
              loop={loop}
              overriding={overrideLoop.isPending}
              resetting={resetGitLoop.isPending}
              onOverride={handleOverride}
              onResetGit={handleResetGit}
            />
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

      {loop.syncedFromGit ? (
        <div className="rounded-md border bg-muted p-4 text-sm">
          This Loop’s configuration is read-only because it is synchronized from{' '}
          <span className="font-medium">{loop.sourceConfigPath}</span>. Override it to edit locally.
          Repository sync will preserve overrides until Reset to Git is chosen.
        </div>
      ) : null}

      <div className="rounded-md border p-4">
        <h2 className="mb-4 text-sm font-semibold">Loop</h2>
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Kind" value={loop.kind} />
          <DetailItem label="Name" value={loop.name} />
          <DetailItem label="Scope" value={loop.scope} />
          <DetailItem label="Configuration source" value={loop.configSource} />
          <DetailItem label="Git source key" value={loop.sourceKey ?? '-'} />
          <DetailItem label="Created" value={formatTimestamp(loop.createdAt)} />
        </dl>
      </div>
      <SourceInfoSection loop={loop} connection={connection} />
      <ConfigInfoSection loop={loop} />
      {!loop.deleted ? <ExecutionPanel key={`${loop.id}:${loop.updatedAt}`} loop={loop} /> : null}
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
              {loop.sourceRepositoryId
                ? `Delete Loop "${loop.name}"? Its Git provenance will be kept so an administrator can restore it explicitly.`
                : `Delete Loop "${loop.name}"? This cannot be undone.`}
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

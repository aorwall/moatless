import type { TaskResponse } from '@moatless/api-client'
import { useNavigate } from '@tanstack/react-router'
import {
  Bell,
  BellOff,
  ExternalLink,
  GitBranch,
  Globe,
  Lock,
  RotateCcw,
  XCircle,
} from 'lucide-react'
import { toast } from 'sonner'
import { formatTimestamp } from '@/features/task-connections'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskStatisticsQuery } from '@/features/tasks/hooks/use-task-queries'
import {
  useCloseTask,
  useReopenTask,
  useTasksList,
  useToggleTaskFollow,
  useUpdateTaskTtl,
  useUpdateTaskVisibility,
} from '@/features/tasks/hooks/use-tasks'
import { formatRelativeTime } from '@/features/tasks/utils/format-relative-time'
import { getIdleStopDeadlineMs } from '@/features/tasks/utils/task-ttl'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/lib/components/ui/alert-dialog'
import { Badge } from '@/lib/components/ui/badge'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { useNow } from '@/lib/hooks/use-now'
import { cn } from '@/lib/utils'

/** Auto close: days of inactivity before the task is closed (0 = never). */
const AUTO_CLOSE_PRESETS: Array<{ label: string; days: number }> = [
  { label: '1 day', days: 1 },
  { label: '3 days', days: 3 },
  { label: '7 days', days: 7 },
  { label: '30 days', days: 30 },
  { label: 'Never', days: 0 },
]

interface TaskInfoSectionProps {
  task: TaskResponse
}

function TaskStatusControls({
  taskId,
  status,
}: {
  taskId: string
  status: TaskResponse['status']
}) {
  const navigate = useNavigate()
  const closeTask = useCloseTask(taskId, {
    onSuccess: () => navigate({ to: '/' }),
    onError: (err) => toast.error('Failed to close task', { description: err.message }),
  })
  const reopenTask = useReopenTask(taskId, {
    onSuccess: () => toast.success('Task reopened'),
    onError: (err) => toast.error('Failed to reopen task', { description: err.message }),
  })
  const isClosed = status === 'closed'
  const childrenQuery = useTasksList(
    { parentTaskId: taskId, limit: 50 },
    { enabled: !isClosed, staleTime: 30_000 },
  )
  const childCount = childrenQuery.data?.pagination.total
  const childCountKnown = childCount !== undefined && !childrenQuery.isError
  const hasChildren = childCountKnown && childCount > 0
  const closeDisabled = closeTask.isPending || !childCountKnown
  const closeButton = (
    <button
      type="button"
      disabled={closeDisabled}
      onClick={hasChildren ? undefined : () => closeTask.mutate()}
      title={
        childCountKnown
          ? hasChildren
            ? 'Close task and child tasks'
            : 'Close task'
          : 'Checking child tasks'
      }
      className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs text-destructive transition-colors hover:bg-destructive/10 disabled:opacity-50"
    >
      <XCircle className="h-3 w-3" />
      Close
    </button>
  )

  return (
    <div className="flex items-center gap-2">
      <Badge variant={isClosed ? 'secondary' : 'default'} className="h-5 text-xs">
        {status}
      </Badge>
      {isClosed ? (
        <button
          type="button"
          disabled={reopenTask.isPending}
          onClick={() => reopenTask.mutate()}
          title="Reopen task to continue chatting"
          className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs transition-colors hover:bg-muted disabled:opacity-50"
        >
          <RotateCcw className="h-3 w-3" />
          Reopen
        </button>
      ) : hasChildren ? (
        <AlertDialog>
          <AlertDialogTrigger asChild>{closeButton}</AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Close parent task?</AlertDialogTitle>
              <AlertDialogDescription>
                Closing this task will also close {childCount}{' '}
                {childCount === 1 ? 'child task' : 'child tasks'} and stop their sandboxes.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={() => closeTask.mutate()}
              >
                Close all
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : (
        closeButton
      )}
    </div>
  )
}

export function TaskInfoSection({ task }: TaskInfoSectionProps) {
  const now = useNow(60_000)
  const taskId = useTaskId()
  const updateVisibility = useUpdateTaskVisibility(taskId)
  const updateTtl = useUpdateTaskTtl(taskId)
  const statisticsQuery = useTaskStatisticsQuery(taskId)
  const toggleFollow = useToggleTaskFollow(taskId, {
    onError: (err) => toast.error('Failed to update follow', { description: err.message }),
  })
  const isClosed = task.status === 'closed'
  const totalCost = task.totalCostUsd ?? 0
  const createdBy = task.createdByDisplayName ?? task.createdBy

  return (
    <dl className="grid grid-cols-2 gap-x-6 gap-y-3 px-3 py-3">
      {task.repositoryName && (
        <Field label="Repository">
          <Truncated value={task.repositoryName} />
        </Field>
      )}
      {task.branch && (
        <Field label="Branch">
          <span className="flex items-center gap-1 font-mono" title={task.branch}>
            <GitBranch className="h-3 w-3 shrink-0 text-muted-foreground" />
            <span className="truncate">{task.branch}</span>
          </span>
        </Field>
      )}
      <Field label="Status">
        <TaskStatusControls taskId={taskId} status={task.status} />
      </Field>
      <Field label="Visibility">
        <button
          type="button"
          disabled={updateVisibility.isPending}
          onClick={() =>
            updateVisibility.mutate(task.visibility === 'public' ? 'private' : 'public')
          }
          title="Toggle visibility"
          className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs capitalize transition-colors hover:bg-muted disabled:opacity-50"
        >
          {task.visibility === 'public' ? (
            <Globe className="h-3 w-3" />
          ) : (
            <Lock className="h-3 w-3" />
          )}
          {task.visibility}
        </button>
      </Field>
      <Field label="Scope">
        <span className="capitalize">{task.scope}</span>
      </Field>
      <Field label="Agent type">
        <Truncated value={task.agentType} />
      </Field>
      <Field label="Model">
        <Truncated
          value={
            statisticsQuery.isLoading ? 'Loading' : (statisticsQuery.data?.model ?? 'Not recorded')
          }
        />
      </Field>
      <Field label="Follow">
        <button
          type="button"
          disabled={toggleFollow.isPending}
          onClick={() => toggleFollow.mutate(!task.isFollowing)}
          title={
            task.isFollowing
              ? 'Unfollow — remove from your task list'
              : 'Follow — add to your task list'
          }
          className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs transition-colors hover:bg-muted disabled:opacity-50"
        >
          {task.isFollowing ? (
            <>
              <Bell className="h-3 w-3" />
              Following
            </>
          ) : (
            <>
              <BellOff className="h-3 w-3" />
              Follow
            </>
          )}
        </button>
      </Field>
      {createdBy && (
        <Field label="Created by">
          <Truncated value={createdBy} />
        </Field>
      )}
      <Field label="Created">
        <Truncated value={formatTimestamp(task.createdAt)} className="tabular-nums" />
      </Field>
      <Field label="Last message">
        <Truncated value={formatTimestamp(task.lastMessageAt)} className="tabular-nums" />
      </Field>
      {task.sandboxStartedAt && (
        <Field label="Sandbox started">
          <Truncated value={formatTimestamp(task.sandboxStartedAt)} className="tabular-nums" />
        </Field>
      )}
      <Field label="Total cost">
        <span className="tabular-nums">${totalCost.toFixed(4)}</span>
      </Field>
      {!isClosed && (
        <CountdownField
          label="Idle stop"
          deadlineMs={getIdleStopDeadlineMs(task)}
          ttlSpanMs={task.ttlMinutes * 60_000}
          expiredLabel="Stopping…"
          now={now}
        />
      )}
      {!isClosed && (
        <Field label="Auto close">
          <Select
            value={String(task.inactiveTtlDays)}
            onValueChange={(value) => updateTtl.mutate({ inactiveTtlDays: Number(value) })}
            disabled={updateTtl.isPending}
          >
            <SelectTrigger size="sm" className="h-7 w-full text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {!AUTO_CLOSE_PRESETS.some((preset) => preset.days === task.inactiveTtlDays) && (
                <SelectItem value={String(task.inactiveTtlDays)}>
                  {task.inactiveTtlDays} days
                </SelectItem>
              )}
              {AUTO_CLOSE_PRESETS.map((preset) => (
                <SelectItem key={preset.days} value={String(preset.days)}>
                  {preset.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
      )}
      {task.siteUrl && (
        <Field label="Site" full>
          <a
            href={task.siteUrl}
            target="_blank"
            rel="noreferrer"
            className="inline-flex max-w-full items-center gap-1 font-medium hover:underline"
          >
            <span className="truncate">{task.siteUrl}</span>
            <ExternalLink className="h-3 w-3 shrink-0 opacity-60" />
          </a>
        </Field>
      )}
      {task.tags.length > 0 && (
        <Field label="Tags" full>
          <div className="flex flex-wrap gap-1">
            {task.tags.map((tag) => (
              <Badge
                key={tag.tagId}
                variant="secondary"
                className="h-5 px-1.5 text-[10px] font-normal"
              >
                {tag.name}
              </Badge>
            ))}
          </div>
        </Field>
      )}
    </dl>
  )
}

/** A label-over-value cell in the details grid. `full` spans both columns. */
function Field({
  label,
  children,
  full,
}: {
  label: string
  children: React.ReactNode
  full?: boolean
}) {
  return (
    <div className={cn('min-w-0', full && 'col-span-2')}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-sm">{children}</dd>
    </div>
  )
}

/** Single-line value that truncates within its grid cell, full text on hover. */
function Truncated({ value, className }: { value: string; className?: string }) {
  return (
    <span className={cn('block truncate', className)} title={value}>
      {value}
    </span>
  )
}

function CountdownField({
  label,
  deadlineMs,
  ttlSpanMs,
  expiredLabel,
  now,
}: {
  label: string
  deadlineMs: number | null
  ttlSpanMs: number
  expiredLabel: string
  now: number
}) {
  if (deadlineMs == null) {
    return null
  }
  const remaining = deadlineMs - now
  const isAmber = remaining <= 0 || remaining <= ttlSpanMs * 0.1
  return (
    <Field label={label}>
      <span className={cn('tabular-nums', isAmber && 'text-amber-500')}>
        {remaining <= 0 ? expiredLabel : formatRelativeTime(deadlineMs, now)}
      </span>
    </Field>
  )
}

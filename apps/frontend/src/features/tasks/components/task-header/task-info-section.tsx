import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { ExternalLink, GitBranch } from 'lucide-react'
import { formatTimestamp } from '@/features/task-connections'
import { formatRelativeTime } from '@/features/tasks/utils/format-relative-time'
import { getAutoCloseDeadlineMs, getIdleStopDeadlineMs } from '@/features/tasks/utils/task-ttl'
import { Badge } from '@/lib/components/ui/badge'
import { useNow } from '@/lib/hooks/use-now'
import { cn } from '@/lib/utils'

interface TaskInfoSectionProps {
  task: TaskResponse
  sandbox?: SandboxStatusFullResponse
}

export function TaskInfoSection({ task, sandbox }: TaskInfoSectionProps) {
  const now = useNow(60_000)
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
        <Badge variant={isClosed ? 'secondary' : 'default'} className="h-5 text-xs">
          {task.status}
        </Badge>
      </Field>
      <Field label="Agent">
        <Truncated value={task.agentType} className="font-mono" />
      </Field>
      <Field label="Visibility">
        <span className="capitalize">{task.visibility}</span>
      </Field>
      <Field label="Scope">
        <span className="capitalize">{task.scope}</span>
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
      {task.shortId && (
        <Field label="Short ID">
          <span className="font-mono">{task.shortId}</span>
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
        <CountdownField
          label="Auto-close"
          deadlineMs={getAutoCloseDeadlineMs(task)}
          ttlSpanMs={task.inactiveTtlDays * 86_400_000}
          expiredLabel="Closing…"
          now={now}
        />
      )}
      <SandboxStatusFields sandbox={sandbox} />
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

function SandboxStatusFields({ sandbox }: { sandbox?: SandboxStatusFullResponse }) {
  if (!sandbox) {
    return null
  }
  return (
    <>
      {sandbox.environmentStatus && (
        <Field label="Environment">
          <Badge
            variant={sandbox.environmentStatus === 'ready' ? 'default' : 'secondary'}
            className="h-5 text-xs"
          >
            {sandbox.environmentStatus}
          </Badge>
        </Field>
      )}
      {sandbox.agentState && (
        <Field label="Agent state">
          <Badge
            variant={sandbox.agentState === 'running' ? 'default' : 'outline'}
            className="h-5 text-xs"
          >
            {sandbox.agentState}
          </Badge>
        </Field>
      )}
      {sandbox.sandboxState && (
        <Field label="Sandbox">
          <Badge
            variant={sandbox.sandboxState === 'exists' ? 'default' : 'destructive'}
            className="h-5 text-xs"
          >
            {sandbox.sandboxState}
          </Badge>
        </Field>
      )}
      {sandbox.environmentError && (
        <div className="col-span-2 rounded bg-destructive/10 p-2 text-xs text-destructive">
          {sandbox.environmentError}
        </div>
      )}
    </>
  )
}

import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { ExternalLink, GitBranch } from 'lucide-react'
import { formatTimestamp } from '@/features/task-connections'
import { formatRelativeTime } from '@/features/tasks/utils/format-relative-time'
import { getAutoCloseDeadlineMs, getIdleStopDeadlineMs } from '@/features/tasks/utils/task-ttl'
import { Badge } from '@/lib/components/ui/badge'
import { useNow } from '@/lib/hooks/use-now'

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
    <div className="space-y-2 px-2 py-2 text-xs">
      {task.repositoryName && (
        <Row label="Repository">
          <span className="truncate max-w-[200px]" title={task.repositoryName}>
            {task.repositoryName}
          </span>
        </Row>
      )}
      {task.branch && (
        <Row label="Branch">
          <span
            className="flex items-center gap-1 font-mono truncate max-w-[180px]"
            title={task.branch}
          >
            <GitBranch className="h-3 w-3 flex-shrink-0 text-muted-foreground" />
            <span className="truncate">{task.branch}</span>
          </span>
        </Row>
      )}
      <Row label="Status">
        <Badge variant={isClosed ? 'secondary' : 'default'} className="h-5 text-xs">
          {task.status}
        </Badge>
      </Row>
      <Row label="Agent">
        <span className="font-mono">{task.agentType}</span>
      </Row>
      <Row label="Visibility">
        <span className="capitalize">{task.visibility}</span>
      </Row>
      <Row label="Scope">
        <span className="capitalize">{task.scope}</span>
      </Row>
      {task.teamId && (
        <Row label="Team">
          <span className="font-mono truncate max-w-[160px]" title={task.teamId}>
            {task.teamId}
          </span>
        </Row>
      )}
      {createdBy && (
        <Row label="Created by">
          <span className="truncate max-w-[160px]" title={createdBy}>
            {createdBy}
          </span>
        </Row>
      )}
      <Row label="Created">
        <span className="tabular-nums">{formatTimestamp(task.createdAt)}</span>
      </Row>
      <Row label="Last message">
        <span className="tabular-nums">{formatTimestamp(task.lastMessageAt)}</span>
      </Row>
      {task.sandboxStartedAt && (
        <Row label="Sandbox started">
          <span className="tabular-nums">{formatTimestamp(task.sandboxStartedAt)}</span>
        </Row>
      )}
      {task.shortId && (
        <Row label="Short ID">
          <span className="font-mono">{task.shortId}</span>
        </Row>
      )}
      {task.siteUrl && (
        <Row label="Site">
          <a
            href={task.siteUrl}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 font-medium hover:underline max-w-[200px]"
          >
            <span className="truncate">{task.siteUrl}</span>
            <ExternalLink className="h-3 w-3 shrink-0 opacity-60" />
          </a>
        </Row>
      )}
      <Row label="Total cost">
        <span className="tabular-nums">${totalCost.toFixed(4)}</span>
      </Row>
      {!isClosed && (
        <CountdownRow
          label="Idle stop"
          deadlineMs={getIdleStopDeadlineMs(task)}
          ttlSpanMs={task.ttlMinutes * 60_000}
          expiredLabel="Stopping…"
          now={now}
        />
      )}
      {!isClosed && (
        <CountdownRow
          label="Auto-close"
          deadlineMs={getAutoCloseDeadlineMs(task)}
          ttlSpanMs={task.inactiveTtlDays * 86_400_000}
          expiredLabel="Closing…"
          now={now}
        />
      )}
      <SandboxStatusRows sandbox={sandbox} />
      {task.tags.length > 0 && (
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">Tags</span>
          <div className="flex flex-wrap justify-end gap-1">
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
        </div>
      )}
    </div>
  )
}

function CountdownRow({
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
    <Row label={label}>
      <span className={`tabular-nums ${isAmber ? 'text-amber-500' : ''}`}>
        {remaining <= 0 ? expiredLabel : formatRelativeTime(deadlineMs, now)}
      </span>
    </Row>
  )
}

function SandboxStatusRows({ sandbox }: { sandbox?: SandboxStatusFullResponse }) {
  if (!sandbox) {
    return null
  }
  return (
    <>
      {sandbox.environmentStatus && (
        <Row label="Environment">
          <Badge
            variant={sandbox.environmentStatus === 'ready' ? 'default' : 'secondary'}
            className="h-5 text-xs"
          >
            {sandbox.environmentStatus}
          </Badge>
        </Row>
      )}
      {sandbox.agentState && (
        <Row label="Agent">
          <Badge
            variant={sandbox.agentState === 'running' ? 'default' : 'outline'}
            className="h-5 text-xs"
          >
            {sandbox.agentState}
          </Badge>
        </Row>
      )}
      {sandbox.sandboxState && (
        <Row label="Sandbox">
          <Badge
            variant={sandbox.sandboxState === 'exists' ? 'default' : 'destructive'}
            className="h-5 text-xs"
          >
            {sandbox.sandboxState}
          </Badge>
        </Row>
      )}
      {sandbox.environmentError && (
        <div className="text-xs text-red-600 bg-red-50 p-2 rounded">{sandbox.environmentError}</div>
      )}
    </>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </div>
  )
}

import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
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

  return (
    <div className="space-y-2 px-2 py-2 text-xs">
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

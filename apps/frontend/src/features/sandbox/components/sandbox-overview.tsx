import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { sandboxStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import { useQueries } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { AlertTriangle, ArrowLeft, CheckCircle, Loader2, OctagonX, Server } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { SandboxActionButtons } from './sandbox-action-buttons'

type StatusFilter = 'all' | 'ready' | 'initializing' | 'error' | 'stopped' | 'not_created'

function statusBadgeVariant(
  status: string | undefined,
): 'default' | 'secondary' | 'destructive' | 'outline' {
  switch (status) {
    case 'ready':
      return 'default'
    case 'error':
      return 'destructive'
    case 'initializing':
      return 'secondary'
    default:
      return 'outline'
  }
}

function agentBadgeVariant(
  state: string | undefined,
): 'default' | 'secondary' | 'destructive' | 'outline' {
  switch (state) {
    case 'running':
      return 'default'
    case 'idle':
      return 'secondary'
    // Sandbox is ready but the agent could not be reached over gRPC — surface it
    // as a problem rather than a benign idle/none.
    case 'unknown':
      return 'destructive'
    default:
      return 'outline'
  }
}

interface TaskWithStatus {
  task: TaskResponse
  status?: SandboxStatusFullResponse | undefined
  isLoading: boolean
}

function SandboxTableRow({ task, status, isLoading }: TaskWithStatus) {
  return (
    <TableRow>
      <TableCell>
        <Link
          to="/sandbox/$task-id"
          params={{ 'task-id': task.taskId }}
          className="hover:underline font-medium"
        >
          {task.title || task.name || task.taskId}
        </Link>
        <div className="text-xs text-muted-foreground font-mono">
          {task.shortId || task.taskId.slice(0, 8)}
        </div>
      </TableCell>
      <TableCell>
        {isLoading ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Badge variant={statusBadgeVariant(status?.environmentStatus)}>
            {status?.environmentStatus ?? 'unknown'}
          </Badge>
        )}
      </TableCell>
      <TableCell>
        {isLoading ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Badge variant={agentBadgeVariant(status?.agentState)}>
            {status?.agentState ?? 'unknown'}
          </Badge>
        )}
      </TableCell>
      <TableCell>
        {status?.resources?.cpuUsagePercent != null ? (
          <span className="font-mono text-sm">{status.resources.cpuUsagePercent.toFixed(0)}%</span>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {status?.resources?.memoryUsageMb != null ? (
          <span className="font-mono text-sm">
            {status.resources.memoryUsageMb} MB
            {status.resources.memoryLimitMb != null && (
              <span className="text-muted-foreground"> / {status.resources.memoryLimitMb}</span>
            )}
          </span>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {status?.servers && status.servers.length > 0 ? (
          <div className="flex items-center gap-1">
            <Server className="h-3.5 w-3.5 text-muted-foreground" />
            <span className="text-sm">{status.servers.length}</span>
          </div>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        <span className="text-sm text-muted-foreground">
          {task.repositoryName || task.repositoryId}
        </span>
      </TableCell>
      <TableCell className="text-right">
        <SandboxActionButtons
          taskId={task.taskId}
          environmentStatus={status?.environmentStatus}
          compact
        />
      </TableCell>
    </TableRow>
  )
}

export function SandboxOverview() {
  const [filter, setFilter] = useState<StatusFilter>('all')

  const tasksQuery = useTasksList({ status: 'open', limit: 100 })
  const tasks = tasksQuery.data?.items ?? []

  const statusQueries = useQueries({
    queries: tasks.map((task) => ({
      queryKey: queryKeys.tasks.sandboxStatus(task.taskId),
      queryFn: () => unwrapResponse<SandboxStatusFullResponse>(sandboxStatusHandler(task.taskId)),
      enabled: !!task.taskId,
      staleTime: 10_000,
    })),
  })

  const tasksWithStatus: TaskWithStatus[] = useMemo(
    () =>
      tasks.map((task, i) => ({
        task,
        status: statusQueries[i]?.data,
        isLoading: statusQueries[i]?.isLoading ?? true,
      })),
    [tasks, statusQueries],
  )

  const filteredTasks = useMemo(
    () =>
      filter === 'all'
        ? tasksWithStatus
        : tasksWithStatus.filter((t) => t.status?.environmentStatus === filter),
    [tasksWithStatus, filter],
  )

  const counts = useMemo(() => {
    const c = { ready: 0, error: 0, initializing: 0, stopped: 0, other: 0 }
    for (const t of tasksWithStatus) {
      const s = t.status?.environmentStatus
      if (s === 'ready') {
        c.ready++
      } else if (s === 'error') {
        c.error++
      } else if (s === 'initializing') {
        c.initializing++
      } else if (s === 'stopped') {
        c.stopped++
      } else {
        c.other++
      }
    }
    return c
  }, [tasksWithStatus])

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Link to="/">
            <Button variant="ghost" size="icon" aria-label="Back to home">
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <div>
            <h1 className="text-2xl font-bold">Sandbox Status</h1>
            <p className="text-sm text-muted-foreground">
              Diagnostics for all active sandbox environments
            </p>
          </div>
        </div>
      </div>

      {/* Summary cards */}
      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">Running</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-2">
              <CheckCircle className="h-5 w-5 text-green-500" />
              <span className="text-2xl font-bold">{counts.ready}</span>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">Errors</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-2">
              <OctagonX className="h-5 w-5 text-red-500" />
              <span className="text-2xl font-bold">{counts.error}</span>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              Initializing
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-2">
              <Loader2 className="h-5 w-5 text-yellow-500" />
              <span className="text-2xl font-bold">{counts.initializing}</span>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">Stopped</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-2">
              <AlertTriangle className="h-5 w-5 text-muted-foreground" />
              <span className="text-2xl font-bold">{counts.stopped}</span>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Filters */}
      <div className="flex gap-2">
        {(['all', 'ready', 'error', 'initializing', 'stopped'] as const).map((f) => (
          <Button
            key={f}
            variant={filter === f ? 'default' : 'outline'}
            size="sm"
            onClick={() => setFilter(f)}
          >
            {f === 'all' ? 'All' : f.charAt(0).toUpperCase() + f.slice(1)}
          </Button>
        ))}
      </div>

      {/* Table */}
      {tasksQuery.isLoading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <Card>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Task</TableHead>
                <TableHead>Environment</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>CPU</TableHead>
                <TableHead>Memory</TableHead>
                <TableHead>Servers</TableHead>
                <TableHead>Repository</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredTasks.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8} className="text-center py-8 text-muted-foreground">
                    {filter === 'all' ? 'No active tasks' : `No sandboxes with status: ${filter}`}
                  </TableCell>
                </TableRow>
              ) : (
                filteredTasks.map((item) => <SandboxTableRow key={item.task.taskId} {...item} />)
              )}
            </TableBody>
          </Table>
        </Card>
      )}
    </div>
  )
}

import type {
  ContainerDiagnostics,
  ResourceMetrics,
  RuntimeEvent,
  ServerEntry,
} from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle,
  Clock,
  Cpu,
  ExternalLink,
  HardDrive,
  Loader2,
  Server,
  XCircle,
} from 'lucide-react'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Progress } from '@/lib/components/ui/progress'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { SandboxActionButtons } from './sandbox-action-buttons'

interface SandboxDiagnosticsProps {
  taskId: string
}

function statusBadgeVariant(
  status: string | undefined,
): 'default' | 'secondary' | 'destructive' | 'outline' {
  switch (status) {
    case 'ready':
    case 'running':
    case 'started':
      return 'default'
    case 'error':
    case 'failed':
      return 'destructive'
    case 'initializing':
    case 'starting':
    case 'installing':
      return 'secondary'
    default:
      return 'outline'
  }
}

function severityBadgeVariant(severity: string | undefined | null): 'default' | 'destructive' {
  return severity === 'warning' ? 'destructive' : 'default'
}

function formatUptime(seconds: number): string {
  if (seconds < 60) {
    return `${Math.round(seconds)}s`
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`
  }
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  return `${hours}h ${minutes}m`
}

function containerStateIcon(state: string) {
  switch (state) {
    case 'running':
      return <CheckCircle className="h-4 w-4 text-green-500" />
    case 'waiting':
      return <Clock className="h-4 w-4 text-yellow-500" />
    case 'terminated':
      return <XCircle className="h-4 w-4 text-red-500" />
    default:
      return <AlertTriangle className="h-4 w-4 text-muted-foreground" />
  }
}

// ── Overview Tab ──────────────────────────────────────────

function OverviewSection({
  environmentStatus,
  environmentError,
  agentState,
  sandboxState,
  sandboxReady,
  canExecute,
}: {
  environmentStatus: string
  environmentError?: string | null
  agentState: string
  sandboxState: string
  sandboxReady: boolean
  canExecute: boolean
}) {
  return (
    <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm font-medium">Environment Status</CardTitle>
        </CardHeader>
        <CardContent>
          <Badge variant={statusBadgeVariant(environmentStatus)} className="text-sm">
            {environmentStatus}
          </Badge>
          {environmentError && (
            <p className="mt-2 text-sm text-red-600 bg-red-50 dark:bg-red-950/30 p-2 rounded">
              {environmentError}
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm font-medium">Agent State</CardTitle>
        </CardHeader>
        <CardContent>
          <Badge variant={statusBadgeVariant(agentState)} className="text-sm">
            {agentState}
          </Badge>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm font-medium">Readiness</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex items-center justify-between text-sm">
            <span>Sandbox State</span>
            <Badge variant={sandboxState === 'exists' ? 'default' : 'outline'}>
              {sandboxState}
            </Badge>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span>Sandbox Ready</span>
            {sandboxReady ? (
              <CheckCircle className="h-4 w-4 text-green-500" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
          <div className="flex items-center justify-between text-sm">
            <span>Can Execute</span>
            {canExecute ? (
              <CheckCircle className="h-4 w-4 text-green-500" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

// ── Servers Tab ──────────────────────────────────────────

function ServersSection({ servers }: { servers: ServerEntry[] }) {
  if (servers.length === 0) {
    return <div className="text-center py-8 text-muted-foreground">No servers configured</div>
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      {servers.map((server) => (
        <Card key={server.name}>
          <CardHeader className="pb-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Server className="h-4 w-4" />
                <CardTitle className="text-sm font-medium">{server.label || server.name}</CardTitle>
              </div>
              <Badge variant={statusBadgeVariant(server.status)}>{server.status}</Badge>
            </div>
          </CardHeader>
          <CardContent className="space-y-2 text-sm">
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Port</span>
              <span className="font-mono">{server.port}</span>
            </div>
            {server.url && (
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">URL</span>
                <a
                  href={server.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center gap-1 text-blue-600 hover:underline"
                >
                  Open <ExternalLink className="h-3 w-3" />
                </a>
              </div>
            )}
            {server.error && (
              <div className="text-red-600 bg-red-50 dark:bg-red-950/30 p-2 rounded text-xs">
                {server.error}
              </div>
            )}
            {server.detail && !server.error && (
              <div className="text-muted-foreground bg-muted/50 p-2 rounded text-xs">
                {server.detail}
              </div>
            )}
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Default</span>
              <span>{server.default ? 'Yes' : 'No'}</span>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}

// ── Containers Tab ──────────────────────────────────────────

function ContainersSection({
  containers,
}: {
  containers: ContainerDiagnostics[] | null | undefined
}) {
  if (!containers || containers.length === 0) {
    return (
      <div className="text-center py-8 text-muted-foreground">
        No container diagnostics available
      </div>
    )
  }

  return (
    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Container</TableHead>
            <TableHead>State</TableHead>
            <TableHead>Restarts</TableHead>
            <TableHead>Exit Code</TableHead>
            <TableHead>Reason</TableHead>
            <TableHead>CPU</TableHead>
            <TableHead>Memory</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {containers.map((c) => (
            <TableRow key={c.name}>
              <TableCell className="font-mono text-sm">{c.name}</TableCell>
              <TableCell>
                <div className="flex items-center gap-1.5">
                  {containerStateIcon(c.state)}
                  <span className="text-sm">{c.state}</span>
                </div>
              </TableCell>
              <TableCell>
                <span className={c.restartCount > 0 ? 'text-orange-600 font-medium' : ''}>
                  {c.restartCount}
                </span>
              </TableCell>
              <TableCell>
                {c.exitCode != null ? (
                  <span className={c.exitCode !== 0 ? 'text-red-600 font-mono' : 'font-mono'}>
                    {c.exitCode}
                  </span>
                ) : (
                  <span className="text-muted-foreground">-</span>
                )}
                {c.lastExitCode != null && c.lastExitCode !== c.exitCode && (
                  <span className="text-xs text-muted-foreground ml-1">
                    (prev: {c.lastExitCode})
                  </span>
                )}
              </TableCell>
              <TableCell>
                {c.terminatedReason || c.waitingReason || c.lastTerminatedReason ? (
                  <div className="space-y-0.5">
                    {c.terminatedReason && (
                      <Badge variant="destructive" className="text-xs">
                        {c.terminatedReason}
                      </Badge>
                    )}
                    {c.waitingReason && (
                      <Badge variant="secondary" className="text-xs">
                        {c.waitingReason}
                      </Badge>
                    )}
                    {c.lastTerminatedReason && c.lastTerminatedReason !== c.terminatedReason && (
                      <div className="text-xs text-muted-foreground">
                        prev: {c.lastTerminatedReason}
                      </div>
                    )}
                  </div>
                ) : (
                  <span className="text-muted-foreground">-</span>
                )}
              </TableCell>
              <TableCell>
                {c.cpuMillicores != null ? (
                  <span className="font-mono text-sm">{c.cpuMillicores}m</span>
                ) : (
                  <span className="text-muted-foreground">-</span>
                )}
              </TableCell>
              <TableCell>
                {c.memoryMb != null ? (
                  <span className="font-mono text-sm">{c.memoryMb} MB</span>
                ) : (
                  <span className="text-muted-foreground">-</span>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Card>
  )
}

// ── Resources Tab ──────────────────────────────────────────

function ResourcesSection({ resources }: { resources: ResourceMetrics | null | undefined }) {
  if (!resources) {
    return (
      <div className="text-center py-8 text-muted-foreground">No resource metrics available</div>
    )
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader className="pb-2">
          <div className="flex items-center gap-2">
            <HardDrive className="h-4 w-4" />
            <CardTitle className="text-sm font-medium">Memory</CardTitle>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex justify-between text-sm">
            <span className="text-muted-foreground">Usage</span>
            <span className="font-mono">
              {resources.memoryUsageMb} MB
              {resources.memoryLimitMb != null && ` / ${resources.memoryLimitMb} MB`}
            </span>
          </div>
          {resources.memoryUsagePercent != null && (
            <div className="space-y-1">
              <Progress
                value={resources.memoryUsagePercent}
                className={resources.memoryUsagePercent > 90 ? '[&>div]:bg-red-500' : ''}
              />
              <div className="text-xs text-muted-foreground text-right">
                {resources.memoryUsagePercent.toFixed(1)}%
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <div className="flex items-center gap-2">
            <Cpu className="h-4 w-4" />
            <CardTitle className="text-sm font-medium">CPU</CardTitle>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {resources.cpuUsagePercent != null ? (
            <>
              <div className="flex justify-between text-sm">
                <span className="text-muted-foreground">Usage</span>
                <span className="font-mono">{resources.cpuUsagePercent.toFixed(1)}%</span>
              </div>
              <Progress
                value={resources.cpuUsagePercent}
                className={resources.cpuUsagePercent > 90 ? '[&>div]:bg-red-500' : ''}
              />
            </>
          ) : (
            <div className="text-sm text-muted-foreground">Not available</div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-2">
          <div className="flex items-center gap-2">
            <Clock className="h-4 w-4" />
            <CardTitle className="text-sm font-medium">Runtime</CardTitle>
          </div>
        </CardHeader>
        <CardContent className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Uptime</span>
            <span className="font-mono">{formatUptime(resources.uptimeSeconds)}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">Event Loop Latency</span>
            <span className="font-mono">{resources.eventLoopLatencyMs.toFixed(1)} ms</span>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

// ── Events Tab ──────────────────────────────────────────

function EventsSection({ events }: { events: RuntimeEvent[] | null | undefined }) {
  if (!events || events.length === 0) {
    return <div className="text-center py-8 text-muted-foreground">No runtime events</div>
  }

  return (
    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Reason</TableHead>
            <TableHead>Message</TableHead>
            <TableHead>Severity</TableHead>
            <TableHead>Count</TableHead>
            <TableHead>Last Seen</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((event, i) => (
            <TableRow key={`${event.reason}-${i}`}>
              <TableCell className="font-mono text-sm">{event.reason}</TableCell>
              <TableCell className="text-sm max-w-md truncate">{event.message}</TableCell>
              <TableCell>
                <Badge variant={severityBadgeVariant(event.severity)}>
                  {event.severity ?? 'normal'}
                </Badge>
              </TableCell>
              <TableCell>{event.count ?? 1}</TableCell>
              <TableCell className="text-sm text-muted-foreground">
                {event.lastTimestamp ? new Date(event.lastTimestamp).toLocaleTimeString() : '-'}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Card>
  )
}

// ── Main Component ──────────────────────────────────────────

export function SandboxDiagnostics({ taskId }: SandboxDiagnosticsProps) {
  const taskQuery = useTaskDetailQuery(taskId)
  const sandboxQuery = useSandboxStatus(taskId, taskQuery.data, { refetchInterval: 5000 })

  const task = taskQuery.data
  const status = sandboxQuery.data

  if (taskQuery.isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!task) {
    return <div className="text-center py-12 text-muted-foreground">Task not found</div>
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Link to="/sandbox">
            <Button variant="ghost" size="icon">
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <div>
            <h1 className="text-2xl font-bold">{task.title || task.name || taskId}</h1>
            <p className="text-sm text-muted-foreground font-mono">{taskId}</p>
          </div>
        </div>
        <SandboxActionButtons taskId={taskId} environmentStatus={status?.environmentStatus} />
      </div>

      {sandboxQuery.isLoading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      ) : !status ? (
        <div className="text-center py-12 text-muted-foreground">No sandbox status available</div>
      ) : (
        <Tabs defaultValue="overview">
          <TabsList>
            <TabsTrigger value="overview">Overview</TabsTrigger>
            <TabsTrigger value="servers">
              Servers
              {status.servers.length > 0 && (
                <Badge variant="secondary" className="ml-1.5 text-xs">
                  {status.servers.length}
                </Badge>
              )}
            </TabsTrigger>
            <TabsTrigger value="containers">
              Containers
              {status.containerDiagnostics && status.containerDiagnostics.length > 0 && (
                <Badge variant="secondary" className="ml-1.5 text-xs">
                  {status.containerDiagnostics.length}
                </Badge>
              )}
            </TabsTrigger>
            <TabsTrigger value="resources">Resources</TabsTrigger>
            <TabsTrigger value="events">
              Events
              {status.runtimeEvents && status.runtimeEvents.length > 0 && (
                <Badge variant="secondary" className="ml-1.5 text-xs">
                  {status.runtimeEvents.length}
                </Badge>
              )}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="overview" className="mt-4">
            <OverviewSection
              environmentStatus={status.environmentStatus}
              environmentError={status.environmentError}
              agentState={status.agentState}
              sandboxState={status.sandboxState}
              sandboxReady={status.sandboxReady}
              canExecute={status.canExecute}
            />
          </TabsContent>

          <TabsContent value="servers" className="mt-4">
            <ServersSection servers={status.servers} />
          </TabsContent>

          <TabsContent value="containers" className="mt-4">
            <ContainersSection containers={status.containerDiagnostics} />
          </TabsContent>

          <TabsContent value="resources" className="mt-4">
            <ResourcesSection resources={status.resources} />
          </TabsContent>

          <TabsContent value="events" className="mt-4">
            <EventsSection events={status.runtimeEvents} />
          </TabsContent>
        </Tabs>
      )}
    </div>
  )
}

import type { TaskTriggerResponse, TriggerRecentTaskItem } from '@moatless/api-client'
import { createFileRoute, Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { AdminPage } from '@/features/admin'
import {
  useProviderConnections,
  useTaskRoutes,
  useTaskTrigger,
  useTriggerRecentTasks,
} from '@/features/integrations/hooks/use-integrations'
import { DetailItem } from '@/features/settings/components/detail-item'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
}

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
}

function getSourceLabel(trigger: { sourceName?: string | null; sourceMatcher: string }): string {
  return trigger.sourceName || trigger.sourceMatcher
}

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

export const Route = createFileRoute('/admin/integrations/triggers/$trigger-id/')({
  component: TriggerDetailPage,
})

function TriggerInfoSection({
  trigger,
  connectionAccountId,
}: {
  trigger: TaskTriggerResponse
  connectionAccountId: string | undefined
}) {
  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Trigger</h2>
      <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <DetailItem label="Provider" value={getProviderLabel(trigger.provider)} />
        <DetailItem label="Source matcher" value={trigger.sourceMatcher} />
        <DetailItem label="Source name" value={trigger.sourceName ?? '-'} />
        <DetailItem label="Connection" value={connectionAccountId ?? trigger.connectionId} />
        <div>
          <dt className="text-xs font-medium text-muted-foreground">Active</dt>
          <dd className="mt-1 text-sm">
            <Badge variant={trigger.active ? 'default' : 'secondary'}>
              {trigger.active ? 'Active' : 'Inactive'}
            </Badge>
          </dd>
        </div>
        <DetailItem label="Allow direct send" value={trigger.allowDirectSend ? 'Yes' : 'No'} />
        <DetailItem label="Close task on close" value={trigger.closeTaskOnClose ? 'Yes' : 'No'} />
        <DetailItem label="Notify on success" value={trigger.notifyOnSuccess ? 'Yes' : 'No'} />
        <DetailItem label="Created" value={formatTimestamp(trigger.createdAt)} />
      </dl>
    </div>
  )
}

function RouteInfoSection({ routeId }: { routeId: string }) {
  const { data: routes, isLoading } = useTaskRoutes()
  const route = routes?.find((r) => r.id === routeId)

  if (isLoading) {
    return (
      <div className="rounded-md border p-4">
        <h2 className="mb-4 text-sm font-semibold">Route</h2>
        <Loader2 className="h-4 w-4 animate-spin" />
      </div>
    )
  }

  return (
    <div className="rounded-md border p-4">
      <h2 className="mb-4 text-sm font-semibold">Route</h2>
      {!route ? (
        <p className="text-sm text-muted-foreground">Route not found.</p>
      ) : (
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div>
            <dt className="text-xs font-medium text-muted-foreground">Repository</dt>
            <dd className="mt-1 text-sm">{route.repositoryId}</dd>
          </div>
          <DetailItem label="Routing mode" value={getRoutingModeLabel(route.routingMode)} />
          <DetailItem label="Task name" value={route.taskName ?? '-'} />
          <DetailItem label="Tags count" value={String(route.tagIds.length)} />
          <div>
            <dt className="text-xs font-medium text-muted-foreground">Prompt template</dt>
            <dd className="mt-1 text-sm">
              <Badge variant={route.promptTemplate ? 'default' : 'secondary'}>
                {route.promptTemplate ? 'Configured' : 'None'}
              </Badge>
            </dd>
          </div>
        </dl>
      )}
    </div>
  )
}

function RecentTasksSection({ items }: { items: TriggerRecentTaskItem[] }) {
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
            <TableCell className="text-sm">{item.binding.subjectKey}</TableCell>
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

  const { data: trigger, isLoading: triggerLoading } = useTaskTrigger(triggerId)
  const { data: connections } = useProviderConnections()
  const { data: recentTasks, isLoading: tasksLoading } = useTriggerRecentTasks(triggerId, {
    limit: 10,
  })

  const connection = trigger ? connections?.find((c) => c.id === trigger.connectionId) : undefined

  return (
    <AdminPage
      title={trigger ? getSourceLabel(trigger) : 'Trigger'}
      description={trigger ? `${getProviderLabel(trigger.provider)} trigger` : undefined}
      backTo={{ to: '/admin/integrations/triggers', label: 'Back to triggers' }}
      actions={
        trigger ? (
          <Button asChild>
            <Link
              to="/admin/integrations/triggers/$trigger-id/edit"
              params={{ 'trigger-id': trigger.id }}
            >
              Edit
            </Link>
          </Button>
        ) : undefined
      }
    >
      {triggerLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : !trigger ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Trigger not found</p>
          <p className="text-sm">This trigger may have been deleted.</p>
        </div>
      ) : (
        <div className="space-y-6">
          <TriggerInfoSection
            trigger={trigger}
            connectionAccountId={connection?.externalAccountId}
          />
          <RouteInfoSection routeId={trigger.routeId} />
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
        </div>
      )}
    </AdminPage>
  )
}

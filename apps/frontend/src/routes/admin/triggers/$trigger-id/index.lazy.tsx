import type {
  AdapterConnectionResponse,
  AdapterKind,
  TriggerRecentTaskItem,
  TriggerResponse,
} from '@moatless/api-client'
import { createLazyFileRoute, Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { AdminPage } from '@/features/admin'
import {
  useIntegrationConnections,
  useTrigger,
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

type TriggerSource = TriggerResponse['source']

const ADAPTER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

function isAdapterEventSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { adapterKind: AdapterKind }> {
  return 'adapterKind' in source
}

function isScheduleSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { cronExpression: string }> {
  return 'cronExpression' in source
}

function getAdapterLabel(adapter: string): string {
  return ADAPTER_LABELS[adapter] ?? adapter
}

function getSourceLabel(trigger: TriggerResponse): string {
  if (isAdapterEventSource(trigger.source)) {
    return trigger.source.sourceName || trigger.source.sourceMatcher
  }
  if (isScheduleSource(trigger.source)) {
    return trigger.name || trigger.source.cronExpression
  }
  return trigger.name || trigger.id
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

export const Route = createLazyFileRoute('/admin/triggers/$trigger-id/')({
  component: TriggerDetailPage,
})

function SourceInfoSection({
  trigger,
  connection,
}: {
  trigger: TriggerResponse
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

function RuleInfoSection({ trigger }: { trigger: TriggerResponse }) {
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
  const { data: trigger, isLoading: triggerLoading } = useTrigger(triggerId)
  const { data: connections } = useIntegrationConnections()
  const { data: recentTasks, isLoading: tasksLoading } = useTriggerRecentTasks(triggerId, {
    limit: 10,
  })

  const source = trigger?.source
  const connection =
    source && isAdapterEventSource(source)
      ? connections?.find((item) => item.id === source.connectionId)
      : undefined

  return (
    <AdminPage
      title={trigger ? getSourceLabel(trigger) : 'Trigger'}
      description={trigger ? `${trigger.kind} trigger` : undefined}
      backTo={{ to: '/admin/triggers', label: 'Back to triggers' }}
      actions={
        trigger ? (
          <Button asChild>
            <Link to="/admin/triggers/$trigger-id/edit" params={{ 'trigger-id': trigger.id }}>
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
          <div className="rounded-md border p-4">
            <h2 className="mb-4 text-sm font-semibold">Trigger</h2>
            <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <DetailItem label="Kind" value={trigger.kind} />
              <DetailItem label="Name" value={trigger.name ?? '-'} />
              <DetailItem label="Scope" value={trigger.scope} />
              <DetailItem label="Created" value={formatTimestamp(trigger.createdAt)} />
              <div>
                <dt className="text-xs font-medium text-muted-foreground">Active</dt>
                <dd className="mt-1 text-sm">
                  <Badge variant={trigger.active ? 'default' : 'secondary'}>
                    {trigger.active ? 'Active' : 'Inactive'}
                  </Badge>
                </dd>
              </div>
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
        </div>
      )}
    </AdminPage>
  )
}

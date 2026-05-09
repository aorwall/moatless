import { createFileRoute } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { AdminPage } from '@/features/admin'
import { useProviderConnections } from '@/features/integrations/hooks/use-integrations'
import { DetailItem } from '@/features/settings/components/detail-item'

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
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

export const Route = createFileRoute('/admin/integrations/connections/$connection-id/')({
  component: ConnectionDetailPage,
})

function ConnectionDetailPage() {
  const params = Route.useParams()
  const connectionId = params['connection-id']
  const { data: connections, isLoading } = useProviderConnections()
  const connection = connections?.find((item) => item.id === connectionId)

  return (
    <AdminPage
      title={connection?.externalAccountId ?? 'Connection'}
      description={connection ? `${getProviderLabel(connection.provider)} connection` : undefined}
      backTo={{ to: '/admin/integrations/connections', label: 'Back to connections' }}
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : !connection ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Connection not found</p>
          <p className="text-sm">This connection may have been deleted.</p>
        </div>
      ) : (
        <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2 rounded-md border p-4">
          <DetailItem label="Provider" value={getProviderLabel(connection.provider)} />
          <DetailItem label="Connection Kind" value={connection.connectionKind} />
          <DetailItem label="External Account ID" value={connection.externalAccountId} />
          <DetailItem label="Scope" value={connection.scope} />
          <DetailItem label="Team ID" value={connection.teamId ?? '-'} />
          <DetailItem label="Created" value={formatTimestamp(connection.createdAt)} />
          <DetailItem label="Updated" value={formatTimestamp(connection.updatedAt)} />
        </dl>
      )}
    </AdminPage>
  )
}

import { createFileRoute, Link } from '@tanstack/react-router'
import { ArrowLeft, Loader2 } from 'lucide-react'
import { useProviderConnections } from '@/features/settings/hooks/use-integrations'
import { Button } from '@/lib/components/ui/button'

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

export const Route = createFileRoute('/settings/integrations/connections/$connection-id/')({
  component: ConnectionDetailPage,
})

function ConnectionDetailPage() {
  const params = Route.useParams()
  const connectionId = params['connection-id']
  const { data: connections, isLoading } = useProviderConnections()
  const connection = connections?.find((item) => item.id === connectionId)

  if (isLoading) {
    return (
      <div className="flex h-32 items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin" />
      </div>
    )
  }

  if (!connection) {
    return (
      <div className="container mx-auto max-w-3xl px-6 py-6 space-y-4">
        <BackLink />
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <p className="font-medium">Connection not found</p>
          <p className="text-sm">This connection may have been deleted.</p>
        </div>
      </div>
    )
  }

  return (
    <div className="container mx-auto max-w-3xl px-6 py-6 space-y-6">
      <BackLink />

      <div>
        <h1 className="text-2xl font-semibold">{connection.externalAccountId}</h1>
        <p className="text-sm text-muted-foreground">
          {getProviderLabel(connection.provider)} connection
        </p>
      </div>

      <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2 rounded-md border p-4">
        <DetailItem label="Provider" value={getProviderLabel(connection.provider)} />
        <DetailItem label="Connection Kind" value={connection.connectionKind} />
        <DetailItem label="External Account ID" value={connection.externalAccountId} />
        <DetailItem label="Scope" value={connection.scope} />
        <DetailItem label="Team ID" value={connection.teamId ?? '-'} />
        <DetailItem label="Created" value={formatTimestamp(connection.createdAt)} />
        <DetailItem label="Updated" value={formatTimestamp(connection.updatedAt)} />
      </dl>
    </div>
  )
}

function BackLink() {
  return (
    <Button variant="ghost" size="sm" asChild>
      <Link to="/settings/integrations/connections">
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to connections
      </Link>
    </Button>
  )
}

function DetailItem({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
      <dd className="mt-1 text-sm">{value}</dd>
    </div>
  )
}

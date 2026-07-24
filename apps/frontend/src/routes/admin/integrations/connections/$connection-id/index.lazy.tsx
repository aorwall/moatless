import { createLazyFileRoute } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useIntegrationConnections } from '@/features/integrations/hooks/use-integrations'
import { getAdapterLabel } from '@/features/integrations/utils/adapter-display'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailItem } from '@/lib/components/ui/detail-item'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'

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

export const Route = createLazyFileRoute('/admin/integrations/connections/$connection-id/')({
  component: ConnectionDetailPage,
})

function ConnectionDetailPage() {
  const params = Route.useParams()
  const connectionId = params['connection-id']
  const { data: connections, isLoading } = useIntegrationConnections()
  const connection = connections?.find((item) => item.id === connectionId)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!connection) {
    return (
      <DetailNotFound
        title="Connection not found"
        description="This connection may have been deleted."
      />
    )
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <DetailHeader
        title={connection.externalAccountId}
        subtitle={`${getAdapterLabel(connection.adapterKind)} connection`}
      />

      <Card>
        <CardHeader>
          <CardTitle>Details</CardTitle>
          <CardDescription>Connection identity and audit details (read-only).</CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem label="Adapter" value={getAdapterLabel(connection.adapterKind)} />
          <DetailItem label="Connection Kind" value={connection.connectionKind} />
          <DetailItem
            label="External Account ID"
            value={<span className="break-all">{connection.externalAccountId}</span>}
          />
          <DetailItem label="Scope" value={connection.scope} />
          <DetailItem label="Team ID" value={connection.teamId ?? '-'} />
          <DetailItem label="Created" value={formatTimestamp(connection.createdAt)} />
          <DetailItem label="Updated" value={formatTimestamp(connection.updatedAt)} />
        </CardContent>
      </Card>
    </div>
  )
}

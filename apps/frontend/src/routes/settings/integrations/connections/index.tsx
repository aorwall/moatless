import { createFileRoute, Link } from '@tanstack/react-router'
import { Loader2, Plug, Plus } from 'lucide-react'
import { useState } from 'react'
import { ConnectionFormDialog } from '@/features/settings/components/connection-form-dialog'
import { SettingsPage } from '@/features/settings/components/settings-page'
import { useProviderConnections } from '@/features/settings/hooks/use-integrations'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
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

export const Route = createFileRoute('/settings/integrations/connections/')({
  component: ConnectionsListPage,
})

function ConnectionsListPage() {
  const { data: connections, isLoading, error } = useProviderConnections()
  const [dialogOpen, setDialogOpen] = useState(false)
  const items = connections ?? []

  return (
    <SettingsPage
      title="Connections"
      description="Provider connections used by triggers to authenticate and route messages."
      actions={
        <Button onClick={() => setDialogOpen(true)} size="sm">
          <Plus className="mr-2 h-4 w-4" />
          New Connection
        </Button>
      }
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : error ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>Failed to load connections. Please try again later.</AlertDescription>
        </Alert>
      ) : items.length === 0 ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <Plug className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No connections yet</p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Provider</TableHead>
              <TableHead>External Account ID</TableHead>
              <TableHead>Connection Kind</TableHead>
              <TableHead>Created</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {items.map((connection) => (
              <TableRow key={connection.id}>
                <TableCell>{getProviderLabel(connection.provider)}</TableCell>
                <TableCell className="font-medium">
                  <Link
                    to="/settings/integrations/connections/$connection-id"
                    params={{ 'connection-id': connection.id }}
                    className="hover:underline"
                  >
                    {connection.externalAccountId}
                  </Link>
                </TableCell>
                <TableCell>{connection.connectionKind}</TableCell>
                <TableCell>{formatTimestamp(connection.createdAt)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      <ConnectionFormDialog open={dialogOpen} onOpenChange={setDialogOpen} />
    </SettingsPage>
  )
}

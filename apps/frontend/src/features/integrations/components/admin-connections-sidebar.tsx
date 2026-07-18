import type { AdapterConnectionResponse } from '@moatless/api-client'
import { Plug, Plus } from 'lucide-react'
import { useState } from 'react'
import { useIntegrationConnections } from '@/features/integrations/hooks/use-integrations'
import { getAdapterLabel } from '@/features/integrations/utils/trigger-display'
import { ConnectionFormDialog } from '@/features/settings/components/connection-form-dialog'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (connection: AdapterConnectionResponse) => [
  connection.externalAccountId,
  getAdapterLabel(connection.adapterKind),
  connection.connectionKind,
]
const sortByAccount = (a: AdapterConnectionResponse, b: AdapterConnectionResponse) =>
  a.externalAccountId.localeCompare(b.externalAccountId)

export function AdminConnectionsSidebar() {
  const { data: connections, isLoading, error } = useIntegrationConnections()
  const [search, setSearch] = useState('')
  const [dialogOpen, setDialogOpen] = useState(false)

  const items = useListFilter(connections, search, getSearchText, sortByAccount)

  return (
    <>
      <ListDetailSidebar
        icon={Plug}
        title="Connections"
        count={connections?.length}
        action={
          <Button size="sm" variant="ghost" onClick={() => setDialogOpen(true)}>
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add connection</span>
          </Button>
        }
        search={search}
        onSearchChange={setSearch}
        searchPlaceholder="Search connections..."
        isLoading={isLoading}
        error={error}
        isEmpty={items.length === 0}
        emptyMessage="No connections yet"
      >
        {items.map((connection) => (
          <ListDetailSidebar.Item
            key={connection.id}
            to="/admin/integrations/connections/$connection-id"
            params={{ 'connection-id': connection.id }}
            primary={connection.externalAccountId}
            secondary={getAdapterLabel(connection.adapterKind)}
            trailing={
              <Badge variant="secondary" className="h-4 px-1 text-[10px]">
                {connection.connectionKind}
              </Badge>
            }
          />
        ))}
      </ListDetailSidebar>
      <ConnectionFormDialog open={dialogOpen} onOpenChange={setDialogOpen} />
    </>
  )
}

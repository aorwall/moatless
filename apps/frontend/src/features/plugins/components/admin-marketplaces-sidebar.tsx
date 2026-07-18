import { Link } from '@tanstack/react-router'
import { Loader2, Package, Plus } from 'lucide-react'
import { useState } from 'react'
import { useMarketplaces } from '@/features/plugins/hooks/use-plugins'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'
import type { PluginMarketplace } from '@/lib/types/plugins'

const getSearchText = (marketplace: PluginMarketplace) => [
  marketplace.name,
  marketplace.repoUrl,
  marketplace.description ?? '',
]
const sortByName = (a: PluginMarketplace, b: PluginMarketplace) => a.name.localeCompare(b.name)

function StatusBadge({ status }: { status: PluginMarketplace['status'] }) {
  if (status === 'failed') {
    return (
      <Badge variant="destructive" className="h-4 px-1 text-[10px]">
        failed
      </Badge>
    )
  }
  if (status === 'syncing') {
    return (
      <Badge variant="secondary" className="h-4 gap-1 px-1 text-[10px]">
        <Loader2 className="h-3 w-3 animate-spin" />
        syncing
      </Badge>
    )
  }
  return null
}

export function AdminMarketplacesSidebar() {
  const { data, isLoading, error } = useMarketplaces()
  const [search, setSearch] = useState('')

  const marketplaces = data?.marketplaces
  const items = useListFilter(marketplaces, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={Package}
      title="Marketplaces"
      count={marketplaces?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/marketplaces/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add marketplace</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search marketplaces..."
      isLoading={isLoading}
      error={error}
      isEmpty={items.length === 0}
      emptyMessage="No marketplaces yet"
    >
      {items.map((marketplace) => (
        <ListDetailSidebar.Item
          key={marketplace.id}
          to="/admin/marketplaces/$marketplace-id"
          params={{ 'marketplace-id': marketplace.id }}
          primary={marketplace.name}
          secondary={marketplace.repoUrl}
          trailing={<StatusBadge status={marketplace.status} />}
        />
      ))}
    </ListDetailSidebar>
  )
}

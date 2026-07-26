import type { PluginResponse } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Package, Plus } from 'lucide-react'
import { useState } from 'react'
import { usePlugins } from '@/features/plugins/hooks/use-plugins'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (plugin: PluginResponse) => [plugin.name, plugin.gitUrl]
const sortByName = (a: PluginResponse, b: PluginResponse) => a.name.localeCompare(b.name)

export function AdminPluginsSidebar() {
  const { data: plugins, isLoading, error } = usePlugins()
  const [search, setSearch] = useState('')

  const items = useListFilter(plugins, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={Package}
      title="Plugins"
      count={plugins?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/plugins/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Register plugin</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search plugins..."
      isLoading={isLoading}
      error={error}
      isEmpty={items.length === 0}
      emptyMessage="No plugins yet"
    >
      {items.map((plugin) => (
        <ListDetailSidebar.Item
          key={plugin.id}
          to="/admin/plugins/$plugin-id"
          params={{ 'plugin-id': plugin.id }}
          primary={plugin.name}
          secondary={plugin.gitUrl}
        />
      ))}
    </ListDetailSidebar>
  )
}

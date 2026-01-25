import type { PluginInfo, UpdateRepositoryRequest } from '@moatless/types'
import { Check, Loader2, Package } from 'lucide-react'
import { useMemo } from 'react'
import { usePlugins } from '@/features/plugins/hooks/use-plugins'
import { ConfigurationGroup, ConfigurationItem } from '@/lib/components/configuration'
import { Badge } from '@/lib/components/ui/badge'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { cn } from '@/lib/utils'

interface PluginsGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function PluginsGroup({ formData, onUpdateField }: PluginsGroupProps) {
  const { data, isLoading } = usePlugins()
  const plugins = data?.plugins ?? []
  const selectedPlugins = formData.plugins ?? []

  // Group plugins by marketplace
  const pluginsByMarketplace = useMemo(() => {
    const groups = new Map<string, PluginInfo[]>()
    for (const plugin of plugins) {
      const existing = groups.get(plugin.marketplaceName) ?? []
      existing.push(plugin)
      groups.set(plugin.marketplaceName, existing)
    }
    return groups
  }, [plugins])

  const handleTogglePlugin = (pluginId: string, checked: boolean) => {
    if (checked) {
      onUpdateField('plugins', [...selectedPlugins, pluginId])
    } else {
      onUpdateField(
        'plugins',
        selectedPlugins.filter((p) => p !== pluginId),
      )
    }
  }

  const isPluginSelected = (pluginId: string) => selectedPlugins.includes(pluginId)

  if (isLoading) {
    return (
      <ConfigurationGroup title="Plugins" showSeparator>
        <div className="flex items-center justify-center py-8 text-muted-foreground">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          Loading plugins...
        </div>
      </ConfigurationGroup>
    )
  }

  if (plugins.length === 0) {
    return (
      <ConfigurationGroup title="Plugins" showSeparator>
        <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
          <Package className="h-8 w-8 mb-2 opacity-50" />
          <p className="text-sm">No plugin marketplaces configured</p>
          <p className="text-xs mt-1">Add a marketplace in Settings to enable plugins</p>
        </div>
      </ConfigurationGroup>
    )
  }

  return (
    <ConfigurationGroup title="Plugins" showSeparator>
      <ConfigurationItem
        label="Claude Code Plugins"
        description="Enable plugins to extend Claude Code's capabilities"
      >
        <div className="space-y-4">
          {Array.from(pluginsByMarketplace.entries()).map(
            ([marketplaceName, marketplacePlugins]) => (
              <div key={marketplaceName} className="space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
                    {marketplaceName}
                  </span>
                  <Badge variant="secondary" className="text-xs">
                    {marketplacePlugins.length}{' '}
                    {marketplacePlugins.length === 1 ? 'plugin' : 'plugins'}
                  </Badge>
                </div>
                <div className="space-y-1">
                  {marketplacePlugins.map((plugin) => (
                    <PluginItem
                      key={plugin.id}
                      plugin={plugin}
                      isSelected={isPluginSelected(plugin.id)}
                      onToggle={(checked) => handleTogglePlugin(plugin.id, checked)}
                    />
                  ))}
                </div>
              </div>
            ),
          )}
        </div>
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}

interface PluginItemProps {
  plugin: PluginInfo
  isSelected: boolean
  onToggle: (checked: boolean) => void
}

function PluginItem({ plugin, isSelected, onToggle }: PluginItemProps) {
  const checkboxId = `plugin-${plugin.id}`

  return (
    <button
      type="button"
      className={cn(
        'flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors w-full text-left',
        isSelected
          ? 'border-primary/50 bg-primary/5'
          : 'border-border hover:border-muted-foreground/30 hover:bg-muted/50',
      )}
      onClick={() => onToggle(!isSelected)}
    >
      <Checkbox
        id={checkboxId}
        checked={isSelected}
        onCheckedChange={(checked) => onToggle(checked === true)}
        className="mt-0.5"
        onClick={(e) => e.stopPropagation()}
      />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-sm">{plugin.name}</span>
          {isSelected && <Check className="h-3.5 w-3.5 text-primary" />}
          {plugin.version && (
            <Badge variant="outline" className="text-xs">
              v{plugin.version}
            </Badge>
          )}
        </div>
        {plugin.description && (
          <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">{plugin.description}</p>
        )}
      </div>
    </button>
  )
}

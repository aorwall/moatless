import type { UpdateRepositoryRequest } from '@moatless/api-client'
import { Loader2, Package, Plus, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { usePlugins } from '@/features/plugins/hooks/use-plugins'
import { PluginPickerDialog } from '@/features/repositories/components/plugin-picker-dialog'
import { ConfigurationGroup, ConfigurationItem } from '@/lib/components/configuration'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import type { PluginInfo } from '@/lib/types/plugins'

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
  // plugins from the API is typed as { [key: string]: unknown } | null,
  // but we treat it as string[] for the UI
  const selectedPluginIds = (formData.plugins ?? []) as unknown as string[]
  const [isPickerOpen, setIsPickerOpen] = useState(false)

  // Create a map for quick plugin lookup
  const pluginMap = useMemo(() => {
    const map = new Map<string, PluginInfo>()
    for (const plugin of plugins) {
      map.set(plugin.id, plugin)
    }
    return map
  }, [plugins])

  // Get selected plugin objects
  const selectedPlugins = useMemo(() => {
    return selectedPluginIds
      .map((id) => pluginMap.get(id))
      .filter((plugin): plugin is PluginInfo => plugin !== undefined)
  }, [selectedPluginIds, pluginMap])

  const handleRemovePlugin = (pluginId: string) => {
    const filtered = selectedPluginIds.filter((p) => p !== pluginId)
    onUpdateField('plugins', filtered as unknown as UpdateRepositoryRequest['plugins'])
  }

  const handleConfirmSelection = (newSelection: string[]) => {
    onUpdateField('plugins', newSelection as unknown as UpdateRepositoryRequest['plugins'])
  }

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
        <div className="space-y-3">
          {selectedPlugins.length > 0 ? (
            <div className="space-y-1">
              {selectedPlugins.map((plugin) => (
                <SelectedPluginItem
                  key={plugin.id}
                  plugin={plugin}
                  onRemove={() => handleRemovePlugin(plugin.id)}
                />
              ))}
            </div>
          ) : (
            <div className="flex items-center justify-center py-4 text-muted-foreground border border-dashed rounded-md">
              <p className="text-sm">No plugins selected</p>
            </div>
          )}

          <Button variant="outline" size="sm" onClick={() => setIsPickerOpen(true)}>
            <Plus className="h-4 w-4" />
            Add Plugins
          </Button>
        </div>

        <PluginPickerDialog
          open={isPickerOpen}
          onOpenChange={setIsPickerOpen}
          plugins={plugins}
          selectedPluginIds={selectedPluginIds}
          onConfirm={handleConfirmSelection}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}

interface SelectedPluginItemProps {
  plugin: PluginInfo
  onRemove: () => void
}

function SelectedPluginItem({ plugin, onRemove }: SelectedPluginItemProps) {
  return (
    <div className="flex items-center gap-2 p-2 rounded-md border bg-muted/30">
      <button
        type="button"
        onClick={onRemove}
        className="flex-shrink-0 p-0.5 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
        aria-label={`Remove ${plugin.name}`}
      >
        <X className="h-3.5 w-3.5" />
      </button>
      <div className="flex-1 min-w-0 flex items-center gap-2">
        <span className="font-medium text-sm truncate">{plugin.name}</span>
        {plugin.version && (
          <Badge variant="outline" className="text-xs flex-shrink-0">
            v{plugin.version}
          </Badge>
        )}
      </div>
      {plugin.description && (
        <span className="text-xs text-muted-foreground truncate hidden sm:block max-w-[200px]">
          {plugin.description}
        </span>
      )}
    </div>
  )
}

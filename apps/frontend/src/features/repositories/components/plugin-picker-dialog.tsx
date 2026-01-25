import type { PluginInfo } from '@moatless/types'
import { Check, Package, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Checkbox } from '@/lib/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'

interface PluginPickerDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  plugins: PluginInfo[]
  selectedPluginIds: string[]
  onConfirm: (selectedIds: string[]) => void
}

export function PluginPickerDialog({
  open,
  onOpenChange,
  plugins,
  selectedPluginIds,
  onConfirm,
}: PluginPickerDialogProps) {
  const [searchQuery, setSearchQuery] = useState('')
  const [localSelection, setLocalSelection] = useState<string[]>(selectedPluginIds)

  // Reset local selection when dialog opens
  const handleOpenChange = (newOpen: boolean) => {
    if (newOpen) {
      setLocalSelection(selectedPluginIds)
      setSearchQuery('')
    }
    onOpenChange(newOpen)
  }

  // Filter plugins by search query
  const filteredPlugins = useMemo(() => {
    if (!searchQuery.trim()) {
      return plugins
    }
    const query = searchQuery.toLowerCase()
    return plugins.filter(
      (plugin) =>
        plugin.name.toLowerCase().includes(query) ||
        plugin.description?.toLowerCase().includes(query) ||
        plugin.marketplaceName.toLowerCase().includes(query),
    )
  }, [plugins, searchQuery])

  // Group filtered plugins by marketplace
  const pluginsByMarketplace = useMemo(() => {
    const groups = new Map<string, PluginInfo[]>()
    for (const plugin of filteredPlugins) {
      const existing = groups.get(plugin.marketplaceName) ?? []
      existing.push(plugin)
      groups.set(plugin.marketplaceName, existing)
    }
    return groups
  }, [filteredPlugins])

  const handleTogglePlugin = (pluginId: string) => {
    setLocalSelection((prev) =>
      prev.includes(pluginId) ? prev.filter((id) => id !== pluginId) : [...prev, pluginId],
    )
  }

  const handleConfirm = () => {
    onConfirm(localSelection)
    onOpenChange(false)
  }

  const handleCancel = () => {
    onOpenChange(false)
  }

  const isPluginSelected = (pluginId: string) => localSelection.includes(pluginId)

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Select Plugins</DialogTitle>
          <DialogDescription>
            Choose plugins to enable for this repository. Plugins extend Claude Code's capabilities.
          </DialogDescription>
        </DialogHeader>

        <div className="relative">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search plugins..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="pl-9"
          />
        </div>

        <ScrollArea className="h-[300px] -mx-6 px-6">
          {filteredPlugins.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
              <Package className="h-8 w-8 mb-2 opacity-50" />
              <p className="text-sm">
                {searchQuery ? 'No plugins match your search' : 'No plugins available'}
              </p>
            </div>
          ) : (
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
                        <PluginPickerItem
                          key={plugin.id}
                          plugin={plugin}
                          isSelected={isPluginSelected(plugin.id)}
                          onToggle={() => handleTogglePlugin(plugin.id)}
                        />
                      ))}
                    </div>
                  </div>
                ),
              )}
            </div>
          )}
        </ScrollArea>

        <DialogFooter>
          <Button variant="outline" onClick={handleCancel}>
            Cancel
          </Button>
          <Button onClick={handleConfirm}>
            Done
            {localSelection.length > 0 && (
              <Badge variant="secondary" className="ml-1">
                {localSelection.length}
              </Badge>
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

interface PluginPickerItemProps {
  plugin: PluginInfo
  isSelected: boolean
  onToggle: () => void
}

function PluginPickerItem({ plugin, isSelected, onToggle }: PluginPickerItemProps) {
  const checkboxId = `plugin-picker-${plugin.id}`

  return (
    <button
      type="button"
      className={cn(
        'flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors w-full text-left',
        isSelected
          ? 'border-primary/50 bg-primary/5'
          : 'border-border hover:border-muted-foreground/30 hover:bg-muted/50',
      )}
      onClick={onToggle}
    >
      <Checkbox
        id={checkboxId}
        checked={isSelected}
        onCheckedChange={onToggle}
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

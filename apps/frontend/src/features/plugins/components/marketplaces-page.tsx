import {
  AlertCircle,
  ChevronDown,
  ChevronRight,
  Loader2,
  Package,
  Plus,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useAddMarketplace,
  useMarketplaces,
  usePlugins,
  useRemoveMarketplace,
  useSyncMarketplace,
} from '@/features/plugins/hooks/use-plugins'
import { ConfigurationInput } from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import type { PluginInfo, PluginMarketplace } from '@/lib/types/plugins'
import { getErrorMessage } from '@/lib/utils/error'

export function MarketplacesPage() {
  const {
    data: marketplacesData,
    isLoading: marketplacesLoading,
    error: marketplacesError,
  } = useMarketplaces()
  const { data: pluginsData } = usePlugins()
  const addMarketplace = useAddMarketplace()
  const removeMarketplace = useRemoveMarketplace()
  const syncMarketplace = useSyncMarketplace()

  const [showAddForm, setShowAddForm] = useState(false)
  const [repoUrl, setRepoUrl] = useState('')
  const [expandedMarketplaces, setExpandedMarketplaces] = useState<Set<string>>(new Set())
  const [syncingMarketplaces, setSyncingMarketplaces] = useState<Set<string>>(new Set())

  const marketplaces = marketplacesData?.marketplaces ?? []
  const plugins = pluginsData?.plugins ?? []

  // Group plugins by marketplace
  const pluginsByMarketplace = useMemo(() => {
    const groups = new Map<string, PluginInfo[]>()
    for (const plugin of plugins) {
      const existing = groups.get(plugin.marketplaceId) ?? []
      existing.push(plugin)
      groups.set(plugin.marketplaceId, existing)
    }
    return groups
  }, [plugins])

  const toggleMarketplaceExpanded = (marketplaceId: string) => {
    setExpandedMarketplaces((prev) => {
      const next = new Set(prev)
      if (next.has(marketplaceId)) {
        next.delete(marketplaceId)
      } else {
        next.add(marketplaceId)
      }
      return next
    })
  }

  const handleAddMarketplace = () => {
    if (!repoUrl.trim()) {
      toast.error('Repository URL is required')
      return
    }

    addMarketplace.mutate(
      { repoUrl: repoUrl.trim() },
      {
        onSuccess: () => {
          setRepoUrl('')
          setShowAddForm(false)
          toast.success('Marketplace added successfully')
        },
        onError: (error) => {
          toast.error('Failed to add marketplace', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleRemoveMarketplace = (marketplace: PluginMarketplace) => {
    removeMarketplace.mutate(marketplace.id, {
      onSuccess: () => {
        toast.success(`Marketplace "${marketplace.name}" removed`)
      },
      onError: (error) => {
        toast.error('Failed to remove marketplace', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleSyncMarketplace = (marketplace: PluginMarketplace) => {
    setSyncingMarketplaces((prev) => new Set(prev).add(marketplace.id))

    syncMarketplace.mutate(marketplace.id, {
      onSuccess: () => {
        setSyncingMarketplaces((prev) => {
          const next = new Set(prev)
          next.delete(marketplace.id)
          return next
        })
        toast.success(`Marketplace "${marketplace.name}" synced`)
      },
      onError: (error) => {
        setSyncingMarketplaces((prev) => {
          const next = new Set(prev)
          next.delete(marketplace.id)
          return next
        })
        toast.error('Failed to sync marketplace', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleCancelAdd = () => {
    setShowAddForm(false)
    setRepoUrl('')
  }

  return (
    <AdminPage
      title="Marketplaces"
      description="Manage Claude Code plugin marketplaces from GitHub repositories"
      actions={
        <Button
          onClick={() => setShowAddForm(true)}
          size="sm"
          variant="outline"
          disabled={showAddForm || marketplacesLoading || !!marketplacesError}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Marketplace
        </Button>
      }
    >
      {marketplacesLoading ? (
        <div className="flex items-center justify-center h-32">
          <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
        </div>
      ) : marketplacesError ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>Failed to load marketplaces. Please try again later.</AlertDescription>
        </Alert>
      ) : (
        <div className="space-y-4">
          {/* Add marketplace form */}
          {showAddForm && (
            <Card>
              <CardHeader>
                <CardTitle>Add New Marketplace</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-2">
                  <label htmlFor="repo-url" className="text-sm font-medium">
                    GitHub Repository URL
                  </label>
                  <ConfigurationInput
                    id="repo-url"
                    type="text"
                    placeholder="https://github.com/owner/repo"
                    value={repoUrl}
                    onChange={(e) => setRepoUrl(e.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">
                    Enter the URL of a public GitHub repository containing Claude Code plugins
                  </p>
                </div>
                <div className="flex justify-end gap-2">
                  <Button
                    onClick={handleCancelAdd}
                    variant="outline"
                    disabled={addMarketplace.isPending}
                  >
                    Cancel
                  </Button>
                  <Button onClick={handleAddMarketplace} disabled={addMarketplace.isPending}>
                    {addMarketplace.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                    Add Marketplace
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}

          {/* List of marketplaces */}
          <div className="space-y-3">
            {marketplaces.length === 0 && !showAddForm && (
              <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
                <Package className="h-12 w-12 mb-3 opacity-50" />
                <p className="text-sm font-medium">No marketplaces configured</p>
                <p className="text-xs mt-1">Add a marketplace to enable plugins</p>
              </div>
            )}

            {marketplaces.map((marketplace) => {
              const marketplacePlugins = pluginsByMarketplace.get(marketplace.id) ?? []
              const isExpanded = expandedMarketplaces.has(marketplace.id)
              const isSyncing = syncingMarketplaces.has(marketplace.id)

              return (
                <MarketplaceCard
                  key={marketplace.id}
                  marketplace={marketplace}
                  plugins={marketplacePlugins}
                  isExpanded={isExpanded}
                  isSyncing={isSyncing}
                  onToggleExpand={() => toggleMarketplaceExpanded(marketplace.id)}
                  onSync={() => handleSyncMarketplace(marketplace)}
                  onRemove={() => handleRemoveMarketplace(marketplace)}
                />
              )
            })}
          </div>
        </div>
      )}
    </AdminPage>
  )
}

interface MarketplaceCardProps {
  marketplace: PluginMarketplace
  plugins: PluginInfo[]
  isExpanded: boolean
  isSyncing: boolean
  onToggleExpand: () => void
  onSync: () => void
  onRemove: () => void
}

function MarketplaceCard({
  marketplace,
  plugins,
  isExpanded,
  isSyncing,
  onToggleExpand,
  onSync,
  onRemove,
}: MarketplaceCardProps) {
  const isFailed = marketplace.status === 'failed'
  const isSyncingStatus = marketplace.status === 'syncing' || isSyncing

  return (
    <Card>
      <CardHeader className="pb-2">
        <div className="flex items-start justify-between gap-4">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <CardTitle className="text-base font-medium">{marketplace.name}</CardTitle>
              {isFailed && (
                <Badge variant="destructive" className="text-xs">
                  Failed
                </Badge>
              )}
              {isSyncingStatus && (
                <Badge variant="secondary" className="text-xs">
                  <Loader2 className="h-3 w-3 animate-spin mr-1" />
                  Syncing
                </Badge>
              )}
            </div>
            {marketplace.description && (
              <p className="text-sm text-muted-foreground mt-1">{marketplace.description}</p>
            )}
            <p className="text-xs text-muted-foreground mt-1 font-mono truncate">
              {marketplace.repoUrl}
            </p>
            {marketplace.lastSyncedAt && !isFailed && (
              <p className="text-xs text-muted-foreground mt-1">
                Last synced: {new Date(marketplace.lastSyncedAt).toLocaleString()}
              </p>
            )}
          </div>
          <div className="flex gap-2 shrink-0">
            <Button onClick={onSync} size="sm" variant="outline" disabled={isSyncingStatus}>
              <RefreshCw className={`h-4 w-4 ${isSyncingStatus ? 'animate-spin' : ''}`} />
            </Button>
            <Button onClick={onRemove} size="sm" variant="destructive">
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="pt-0">
        {isFailed ? (
          <Alert variant="destructive" className="mt-2">
            <AlertCircle className="h-4 w-4" />
            <AlertTitle>Clone Failed</AlertTitle>
            <AlertDescription className="text-sm">
              {marketplace.lastError || 'Failed to clone the marketplace repository.'}
              <p className="mt-1 text-xs">Try syncing again or check the repository URL.</p>
            </AlertDescription>
          </Alert>
        ) : (
          <Collapsible open={isExpanded} onOpenChange={onToggleExpand}>
            <CollapsibleTrigger asChild>
              <Button
                variant="ghost"
                size="sm"
                className="w-full justify-start px-0 hover:bg-transparent"
              >
                {isExpanded ? (
                  <ChevronDown className="h-4 w-4 mr-2" />
                ) : (
                  <ChevronRight className="h-4 w-4 mr-2" />
                )}
                <span className="text-sm">Plugins ({plugins.length})</span>
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div className="space-y-2 mt-2">
                {plugins.length === 0 ? (
                  <p className="text-xs text-muted-foreground italic">
                    No plugins found in this marketplace
                  </p>
                ) : (
                  plugins.map((plugin) => (
                    <div key={plugin.id} className="p-2 rounded-md border bg-muted/30">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium">{plugin.name}</span>
                        {plugin.version && (
                          <Badge variant="outline" className="text-xs">
                            v{plugin.version}
                          </Badge>
                        )}
                      </div>
                      {plugin.description && (
                        <p className="text-xs text-muted-foreground mt-0.5">{plugin.description}</p>
                      )}
                    </div>
                  ))
                )}
              </div>
            </CollapsibleContent>
          </Collapsible>
        )}
      </CardContent>
    </Card>
  )
}

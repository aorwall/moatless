import { useNavigate } from '@tanstack/react-router'
import { AlertCircle, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useMarketplaces,
  usePlugins,
  useRemoveMarketplace,
  useSyncMarketplace,
} from '@/features/plugins/hooks/use-plugins'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailItem } from '@/lib/components/ui/detail-item'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
import type { PluginInfo, PluginMarketplace } from '@/lib/types/plugins'
import { getErrorMessage } from '@/lib/utils/error'

function formatTimestamp(value: Date | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? '-' : date.toLocaleString()
}

export function AdminMarketplaceDetail({ marketplaceId }: { marketplaceId: string }) {
  const { data, isLoading } = useMarketplaces()
  const marketplace = data?.marketplaces.find((item) => item.id === marketplaceId)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!marketplace) {
    return (
      <DetailNotFound
        title="Marketplace not found"
        description="This marketplace may have been removed."
      />
    )
  }

  return <AdminMarketplaceDetailBody marketplace={marketplace} />
}

function AdminMarketplaceDetailBody({ marketplace }: { marketplace: PluginMarketplace }) {
  const navigate = useNavigate()
  const { data: pluginsData } = usePlugins()
  const syncMarketplace = useSyncMarketplace()
  const removeMarketplace = useRemoveMarketplace()
  const [confirmOpen, setConfirmOpen] = useState(false)

  const plugins = (pluginsData?.plugins ?? []).filter(
    (plugin) => plugin.marketplaceId === marketplace.id,
  )

  const isFailed = marketplace.status === 'failed'
  const isSyncing = marketplace.status === 'syncing' || syncMarketplace.isPending

  const handleSync = () => {
    syncMarketplace.mutate(marketplace.id, {
      onSuccess: () => toast.success(`Marketplace "${marketplace.name}" synced`),
      onError: (error) =>
        toast.error('Failed to sync marketplace', { description: getErrorMessage(error) }),
    })
  }

  const handleRemove = () => {
    removeMarketplace.mutate(marketplace.id, {
      onSuccess: async () => {
        toast.success(`Marketplace "${marketplace.name}" removed`)
        setConfirmOpen(false)
        await navigate({ to: '/admin/marketplaces' })
      },
      onError: (error) =>
        toast.error('Failed to remove marketplace', { description: getErrorMessage(error) }),
    })
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        title={marketplace.name}
        subtitle={marketplace.description ?? undefined}
        badges={
          isFailed ? (
            <Badge variant="destructive">failed</Badge>
          ) : isSyncing ? (
            <Badge variant="secondary" className="gap-1">
              <Loader2 className="h-3 w-3 animate-spin" />
              syncing
            </Badge>
          ) : null
        }
        actions={
          <div className="flex gap-2">
            <Button onClick={handleSync} size="sm" variant="outline" disabled={isSyncing}>
              <RefreshCw className={`mr-2 h-4 w-4 ${isSyncing ? 'animate-spin' : ''}`} />
              Sync
            </Button>
            <Button
              onClick={() => setConfirmOpen(true)}
              size="sm"
              variant="destructive"
              disabled={removeMarketplace.isPending}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Remove
            </Button>
          </div>
        }
      />

      {isFailed ? (
        <Alert variant="destructive">
          <AlertCircle className="h-4 w-4" />
          <AlertTitle>Clone Failed</AlertTitle>
          <AlertDescription>
            {marketplace.lastError || 'Failed to clone the marketplace repository.'}
          </AlertDescription>
        </Alert>
      ) : null}

      <Card>
        <CardHeader>
          <CardTitle>Details</CardTitle>
          <CardDescription>Marketplace source and sync status.</CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem
            label="Repository URL"
            value={<span className="break-all font-mono text-xs">{marketplace.repoUrl}</span>}
          />
          <DetailItem label="Owner" value={marketplace.ownerName ?? '-'} />
          <DetailItem label="Status" value={marketplace.status} />
          <DetailItem label="Plugins" value={plugins.length} />
          <DetailItem label="Last synced" value={formatTimestamp(marketplace.lastSyncedAt)} />
          <DetailItem label="Added" value={formatTimestamp(marketplace.createdAt)} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Plugins ({plugins.length})</CardTitle>
          <CardDescription>Plugins provided by this marketplace.</CardDescription>
        </CardHeader>
        <CardContent>
          {plugins.length === 0 ? (
            <p className="text-sm text-muted-foreground">No plugins found in this marketplace.</p>
          ) : (
            <div className="space-y-2">
              {plugins.map((plugin) => (
                <PluginRow key={plugin.id} plugin={plugin} />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove marketplace?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes "{marketplace.name}" and its plugins. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removeMarketplace.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault()
                handleRemove()
              }}
              disabled={removeMarketplace.isPending}
            >
              {removeMarketplace.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : null}
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function PluginRow({ plugin }: { plugin: PluginInfo }) {
  return (
    <div className="rounded-md border bg-muted/30 p-2">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{plugin.name}</span>
        {plugin.version ? (
          <Badge variant="outline" className="text-xs">
            v{plugin.version}
          </Badge>
        ) : null}
      </div>
      {plugin.description ? (
        <p className="mt-0.5 text-xs text-muted-foreground">{plugin.description}</p>
      ) : null}
    </div>
  )
}

import type { PluginResponse } from '@moatless/api-client'
import { useNavigate } from '@tanstack/react-router'
import { Loader2, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { PluginActivationTable } from '@/features/plugins/components/plugin-activation-table'
import { useDeletePlugin, usePlugins } from '@/features/plugins/hooks/use-plugins'
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
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailItem } from '@/lib/components/ui/detail-item'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
import { getErrorMessage } from '@/lib/utils/error'

export function AdminPluginDetail({ pluginId }: { pluginId: string }) {
  const { data: plugins, isLoading } = usePlugins()
  const plugin = plugins?.find((item) => item.id === pluginId)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!plugin) {
    return (
      <DetailNotFound title="Plugin not found" description="This plugin may have been removed." />
    )
  }

  return <AdminPluginDetailBody plugin={plugin} />
}

function AdminPluginDetailBody({ plugin }: { plugin: PluginResponse }) {
  const navigate = useNavigate()
  const [confirmOpen, setConfirmOpen] = useState(false)

  const deletePlugin = useDeletePlugin({
    onSuccess: async () => {
      toast.success(`Plugin "${plugin.name}" deleted`)
      setConfirmOpen(false)
      await navigate({ to: '/admin/plugins' })
    },
    onError: (error) =>
      toast.error('Failed to delete plugin', { description: getErrorMessage(error) }),
  })

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        title={plugin.name}
        actions={
          <Button
            onClick={() => setConfirmOpen(true)}
            size="sm"
            variant="destructive"
            disabled={deletePlugin.isPending}
          >
            <Trash2 className="mr-2 h-4 w-4" />
            Delete
          </Button>
        }
      />

      <Card>
        <CardHeader>
          <CardTitle>Details</CardTitle>
          <CardDescription>Where this plugin is fetched from.</CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <DetailItem
            label="Git URL"
            value={<span className="break-all font-mono text-xs">{plugin.gitUrl}</span>}
          />
          <DetailItem
            label="ID"
            value={<span className="break-all font-mono text-xs">{plugin.id}</span>}
          />
          <DetailItem
            label="Synced version"
            value={
              plugin.syncedVersion ? (
                <span className="break-all font-mono text-xs">{plugin.syncedVersion}</span>
              ) : (
                <span className="text-muted-foreground">Not synced yet</span>
              )
            }
          />
        </CardContent>
      </Card>

      <PluginActivationTable pluginId={plugin.id} pluginName={plugin.name} />

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete plugin?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes "{plugin.name}" from the registry, along with every record saying who
              gets it. Nobody receives it after this. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deletePlugin.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault()
                deletePlugin.mutate(plugin.id)
              }}
              disabled={deletePlugin.isPending}
            >
              {deletePlugin.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

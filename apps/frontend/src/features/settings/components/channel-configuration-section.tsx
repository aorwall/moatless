import { Loader2, MessageSquare, Pencil, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import type { ChannelConfigResponse } from '@/features/settings/api/channels.api'
import { ChannelConfigFormDialog } from '@/features/settings/components/channel-config-form-dialog'
import {
  useChannelConfigs,
  useDeleteChannelConfig,
  useUpdateChannelConfig,
} from '@/features/settings/hooks/use-channels'
import { ConfigurationGroup, ConfigurationSectionWrapper } from '@/lib/components/configuration'
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
import { Switch } from '@/lib/components/ui/switch'
import { getErrorMessage } from '@/lib/utils/error'

const ROUTING_MODE_LABELS: Record<string, string> = {
  thread_based: 'Thread Based',
  per_message: 'Per Message',
  ongoing: 'Ongoing',
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
}

function getChannelDisplayName(config: ChannelConfigResponse): string {
  return config.sourceName ?? config.sourceId
}

export function ChannelConfigurationSection() {
  const { data: configs, isLoading, error } = useChannelConfigs()
  const { data: repos } = useRepositoriesList()
  const deleteConfig = useDeleteChannelConfig()
  const updateConfig = useUpdateChannelConfig()

  const [isFormOpen, setIsFormOpen] = useState(false)
  const [editingConfig, setEditingConfig] = useState<ChannelConfigResponse | null>(null)
  const [configToDelete, setConfigToDelete] = useState<ChannelConfigResponse | null>(null)

  const repoNameMap = new Map(repos?.map((r) => [r.id, r.name]) ?? [])

  const handleEdit = (config: ChannelConfigResponse) => {
    setEditingConfig(config)
    setIsFormOpen(true)
  }

  const handleAdd = () => {
    setEditingConfig(null)
    setIsFormOpen(true)
  }

  const handleToggleActive = (config: ChannelConfigResponse) => {
    updateConfig.mutate(
      {
        id: config.id,
        request: { active: !config.active },
      },
      {
        onError: (err) => {
          toast.error('Failed to update config', {
            description: getErrorMessage(err),
          })
        },
      },
    )
  }

  const confirmDelete = () => {
    if (!configToDelete) {
      return
    }
    deleteConfig.mutate(configToDelete.id, {
      onSuccess: () => {
        toast.success('Channel config deleted')
        setConfigToDelete(null)
      },
      onError: (err) => {
        toast.error('Failed to delete config', {
          description: getErrorMessage(err),
        })
      },
    })
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load channel configs. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  const items = configs ?? []

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Channel Routing" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Route messages from external channels (Slack) to repositories. Each config maps a channel
          to a repository with a routing strategy.
        </p>

        {/* Add Button */}
        <div className="flex justify-end mb-4">
          <Button onClick={handleAdd} size="sm">
            <Plus className="mr-2 h-4 w-4" />
            Add Channel
          </Button>
        </div>

        {/* Config List */}
        {items.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            <MessageSquare className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p>No channel configs yet</p>
            <p className="text-sm">Add a channel to start routing messages</p>
          </div>
        ) : (
          <div className="space-y-3">
            {items.map((config) => (
              <div
                key={config.id}
                className="flex items-center justify-between p-4 border rounded-lg bg-card"
              >
                <div className="flex-1 min-w-0 mr-4">
                  <div className="flex items-center gap-2 flex-wrap">
                    <MessageSquare className="h-4 w-4 text-muted-foreground shrink-0" />
                    <span className="font-medium truncate">{getChannelDisplayName(config)}</span>
                    <Badge variant="secondary" className="text-xs">
                      {config.provider}
                    </Badge>
                    <Badge variant="outline" className="text-xs">
                      {getRoutingModeLabel(config.routingMode)}
                    </Badge>
                    {!config.active && (
                      <Badge variant="destructive" className="text-xs">
                        Inactive
                      </Badge>
                    )}
                  </div>
                  <div className="mt-1 text-sm text-muted-foreground">
                    <span>{repoNameMap.get(config.repositoryId) ?? config.repositoryId}</span>
                  </div>
                </div>

                <div className="flex items-center gap-2 shrink-0">
                  <Switch
                    checked={config.active}
                    onCheckedChange={() => handleToggleActive(config)}
                    aria-label="Toggle active"
                  />
                  <Button variant="ghost" size="sm" onClick={() => handleEdit(config)}>
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setConfigToDelete(config)}
                    className="text-destructive hover:text-destructive hover:bg-destructive/10"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </ConfigurationGroup>

      {/* Add/Edit Dialog */}
      <ChannelConfigFormDialog
        open={isFormOpen}
        onOpenChange={setIsFormOpen}
        editConfig={editingConfig}
      />

      {/* Delete Confirmation */}
      <AlertDialog open={!!configToDelete} onOpenChange={() => setConfigToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Channel Config</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete the channel config for "
              {configToDelete && getChannelDisplayName(configToDelete)}"? Messages from this channel
              will no longer be routed to tasks.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteConfig.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Trash2 className="mr-2 h-4 w-4" />
              )}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfigurationSectionWrapper>
  )
}

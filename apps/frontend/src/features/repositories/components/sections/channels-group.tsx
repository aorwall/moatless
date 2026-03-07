import { Copy, Loader2, Mail, Plus, Slack, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useCreateSlackChannelMapping,
  useDeleteSlackChannelMapping,
  useSlackChannelMappingsForRepository,
} from '@/features/repositories/hooks/use-slack-channel-mappings'
import { useChannelProviders } from '@/features/settings/hooks/use-channels'
import { ConfigurationGroup, ConfigurationItem } from '@/lib/components/configuration'
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { useFeatureFlagsContext } from '@/lib/contexts/feature-flags-context'
import { getErrorMessage } from '@/lib/utils/error'

// TODO: Migrate to new channel config API shape from backend
interface ChannelData {
  email?: { address: string }
}

interface ChannelsGroupProps {
  channels: ChannelData | undefined
  repositoryId: string
}

export function ChannelsGroup({ channels, repositoryId }: ChannelsGroupProps) {
  const { data: channelProviders } = useChannelProviders()
  const isSlackEnabled = channelProviders?.includes('slack') ?? false
  const { getSlackConfig } = useFeatureFlagsContext()
  const slackConfig = getSlackConfig()
  const [showAddSlackDialog, setShowAddSlackDialog] = useState(false)
  const [showDeleteDialog, setShowDeleteDialog] = useState<{
    channelId: string
    teamId: string
  } | null>(null)
  const [channelId, setChannelId] = useState('')
  const [teamId, setTeamId] = useState(slackConfig?.teamId ?? '')

  const { data: slackMappings, isLoading: isLoadingMappings } =
    useSlackChannelMappingsForRepository(repositoryId, {
      enabled: isSlackEnabled,
    })

  const createMapping = useCreateSlackChannelMapping()
  const deleteMapping = useDeleteSlackChannelMapping()

  // Only render if there are any channel integrations available
  const hasEmailChannel = !!channels?.email
  const hasSlackChannel = isSlackEnabled

  if (!(hasEmailChannel || hasSlackChannel)) {
    return null
  }

  const handleCopyEmail = async () => {
    if (!channels?.email) {
      return
    }

    try {
      await navigator.clipboard.writeText(channels.email.address)
      toast.success('Email address copied to clipboard')
    } catch {
      toast.error('Failed to copy email address')
    }
  }

  const handleAddSlackMapping = () => {
    if (!(channelId.trim() && teamId.trim())) {
      toast.error('Please fill in all fields')
      return
    }

    createMapping.mutate(
      {
        channelId: channelId.trim(),
        teamId: teamId.trim(),
        repositoryId,
      },
      {
        onSuccess: () => {
          toast.success('Slack channel mapping added')
          setShowAddSlackDialog(false)
          setChannelId('')
          setTeamId('')
        },
        onError: (error) => {
          toast.error('Failed to add Slack channel mapping', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleDeleteSlackMapping = () => {
    if (!showDeleteDialog) {
      return
    }

    deleteMapping.mutate(
      { channelId: showDeleteDialog.channelId, teamId: showDeleteDialog.teamId },
      {
        onSuccess: () => {
          toast.success('Slack channel mapping removed')
          setShowDeleteDialog(null)
        },
        onError: (error) => {
          toast.error('Failed to remove Slack channel mapping', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  return (
    <ConfigurationGroup title="Channels">
      {hasEmailChannel && channels?.email && (
        <ConfigurationItem
          label="Email Address"
          description="Send emails to this address to create tasks for this repository"
        >
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Mail className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={channels.email.address}
                readOnly
                className="pl-10 font-mono text-sm bg-muted"
              />
            </div>
            <Button variant="outline" size="icon" onClick={handleCopyEmail} title="Copy email">
              <Copy className="h-4 w-4" />
            </Button>
          </div>
        </ConfigurationItem>
      )}

      {hasSlackChannel && (
        <ConfigurationItem
          label="Slack Channels"
          description="Map Slack channels to this repository. Users can mention @moatless without specifying a repo."
        >
          <div className="space-y-3">
            {isLoadingMappings ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                Loading mappings...
              </div>
            ) : slackMappings && slackMappings.length > 0 ? (
              <div className="space-y-2">
                {slackMappings.map((mapping) => (
                  <div
                    key={`${mapping.channelId}-${mapping.teamId}`}
                    className="flex items-center gap-2 p-2 rounded-md bg-muted"
                  >
                    <Slack className="h-4 w-4 text-muted-foreground" />
                    <div className="flex-1 font-mono text-sm">
                      <span className="text-muted-foreground">Channel:</span> {mapping.channelId}
                      <span className="mx-2 text-muted-foreground">|</span>
                      <span className="text-muted-foreground">Team:</span> {mapping.teamId}
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() =>
                        setShowDeleteDialog({
                          channelId: mapping.channelId,
                          teamId: mapping.teamId,
                        })
                      }
                      title="Remove mapping"
                    >
                      <Trash2 className="h-4 w-4 text-destructive" />
                    </Button>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                No Slack channels mapped to this repository.
              </p>
            )}

            <Button variant="outline" size="sm" onClick={() => setShowAddSlackDialog(true)}>
              <Plus className="mr-2 h-4 w-4" />
              Add Slack Channel
            </Button>
          </div>
        </ConfigurationItem>
      )}

      {/* Add Slack Channel Dialog */}
      <Dialog open={showAddSlackDialog} onOpenChange={setShowAddSlackDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Slack Channel Mapping</DialogTitle>
            <DialogDescription>
              Map a Slack channel to this repository. Users can mention @moatless in the mapped
              channel without specifying a repo: prefix.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="channelId">Channel ID</Label>
              <Input
                id="channelId"
                placeholder="C01234567"
                value={channelId}
                onChange={(e) => setChannelId(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Find this in Slack: Right-click channel → View channel details → Copy Channel ID
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="teamId">Team ID (Workspace ID)</Label>
              <Input
                id="teamId"
                placeholder="T01234567"
                value={teamId}
                onChange={(e) => setTeamId(e.target.value)}
                readOnly={!!slackConfig?.teamId}
                className={slackConfig?.teamId ? 'bg-muted' : ''}
              />
              <p className="text-xs text-muted-foreground">
                {slackConfig?.teamId
                  ? 'Pre-configured by system administrator'
                  : 'Find this in Slack URL: app.slack.com/client/T01234567/...'}
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAddSlackDialog(false)}>
              Cancel
            </Button>
            <Button onClick={handleAddSlackMapping} disabled={createMapping.isPending}>
              {createMapping.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Adding...
                </>
              ) : (
                'Add Mapping'
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <AlertDialog open={!!showDeleteDialog} onOpenChange={() => setShowDeleteDialog(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove Slack Channel Mapping</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to remove this Slack channel mapping? Users will need to specify
              repo: prefix when mentioning @moatless in this channel.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteMapping.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteSlackMapping}
              disabled={deleteMapping.isPending}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteMapping.isPending ? 'Removing...' : 'Remove'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfigurationGroup>
  )
}

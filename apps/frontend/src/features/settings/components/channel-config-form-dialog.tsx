import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import type { ChannelConfigResponse } from '@/features/settings/api/channels.api'
import {
  useCreateChannelConfig,
  useSlackChannels,
  useUpdateChannelConfig,
} from '@/features/settings/hooks/use-channels'
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Switch } from '@/lib/components/ui/switch'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

interface ChannelConfigFormDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  editConfig?: ChannelConfigResponse | null
  enabledProviders?: string[]
}

interface FormState {
  provider: string
  sourceId: string
  sourceName: string
  repositoryId: string
  routingMode: string
  triggerMode: string
  promptTemplate: string
  active: boolean
  secretToken: string
  botUsername: string
  chatMode: 'dm_wildcard' | 'specific'
}

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
}

function makeInitialForm(defaultProvider: string): FormState {
  return {
    provider: defaultProvider,
    sourceId: '',
    sourceName: '',
    repositoryId: '',
    routingMode: 'thread_based',
    triggerMode: 'mention_only',
    promptTemplate: '',
    active: true,
    secretToken: '',
    botUsername: '',
    chatMode: 'specific',
  }
}

const ROUTING_MODE_DESCRIPTIONS: Record<string, string> = {
  thread_based: 'New messages create tasks. Thread replies continue the existing task.',
  per_message: 'Each message creates a new task. Thread replies still follow existing tasks.',
  ongoing: 'All messages route to a single long-lived task. Auto-creates new when closed.',
}

function formStateFromConfig(config: ChannelConfigResponse): FormState {
  const providerConfig = config.providerConfig as Record<string, unknown> | null
  return {
    provider: config.provider,
    sourceId: config.sourceId,
    sourceName: config.sourceName ?? '',
    repositoryId: config.repositoryId,
    routingMode: config.routingMode,
    triggerMode: (providerConfig?.trigger as string) ?? 'mention_only',
    promptTemplate: config.promptTemplate ?? '',
    active: config.active,
    secretToken: (providerConfig?.secret_token as string) ?? '',
    botUsername: (providerConfig?.bot_username as string) ?? '',
    chatMode: config.sourceId === '*' ? 'dm_wildcard' : 'specific',
  }
}

interface SlackChannelFieldProps {
  sourceId: string
  onChange: (value: string) => void
  disabled: boolean
  slackChannels: Array<{ id: string; name: string }> | undefined
  slackLoading: boolean
  slackError: boolean
}

function SlackChannelField({
  sourceId,
  onChange,
  disabled,
  slackChannels,
  slackLoading,
  slackError,
}: SlackChannelFieldProps) {
  const useDropdown = !(slackError || slackLoading) && slackChannels && slackChannels.length > 0

  return (
    <div className="space-y-2">
      <Label>Channel</Label>
      {slackLoading ? (
        <div className="flex items-center gap-2 h-9 px-3 border rounded-md">
          <Loader2 className="h-4 w-4 animate-spin" />
          <span className="text-sm text-muted-foreground">Loading channels...</span>
        </div>
      ) : useDropdown ? (
        <Select value={sourceId} onValueChange={onChange} disabled={disabled}>
          <SelectTrigger>
            <SelectValue placeholder="Select a channel" />
          </SelectTrigger>
          <SelectContent>
            {slackChannels.map((ch) => (
              <SelectItem key={ch.id} value={ch.id}>
                #{ch.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : (
        <>
          <Input
            value={sourceId}
            onChange={(e) => onChange(e.target.value)}
            placeholder="e.g. C01234567890"
            disabled={disabled}
          />
          <p className="text-xs text-muted-foreground">
            Enter the Slack channel ID. Find it in Slack by right-clicking the channel → View
            channel details → scroll to the bottom.
          </p>
        </>
      )}
    </div>
  )
}

function buildProviderPayload(
  form: FormState,
  slackChannels: Array<{ id: string; name: string }> | undefined,
): { sourceName: string; providerConfig: Record<string, unknown> } {
  if (form.provider === 'slack') {
    const selectedChannel = slackChannels?.find((ch) => ch.id === form.sourceId)
    return {
      sourceName: selectedChannel ? `#${selectedChannel.name}` : form.sourceName || form.sourceId,
      providerConfig: { trigger: form.triggerMode },
    }
  }
  if (form.provider === 'telegram') {
    const providerConfig: Record<string, unknown> = {}
    if (form.secretToken) {
      providerConfig.secret_token = form.secretToken
    }
    if (form.botUsername) {
      providerConfig.bot_username = form.botUsername
    }
    const sourceName =
      form.chatMode === 'dm_wildcard' ? 'All Direct Messages' : form.sourceName || form.sourceId
    return { sourceName, providerConfig }
  }
  return { sourceName: form.sourceName || form.sourceId, providerConfig: {} }
}

interface TelegramFieldsProps {
  sourceId: string
  secretToken: string
  botUsername: string
  chatMode: 'dm_wildcard' | 'specific'
  onSourceIdChange: (value: string) => void
  onSecretTokenChange: (value: string) => void
  onBotUsernameChange: (value: string) => void
  onChatModeChange: (value: 'dm_wildcard' | 'specific') => void
  disabled: boolean
}

function TelegramFields({
  sourceId,
  secretToken,
  botUsername,
  chatMode,
  onSourceIdChange,
  onSecretTokenChange,
  onBotUsernameChange,
  onChatModeChange,
  disabled,
}: TelegramFieldsProps) {
  return (
    <>
      <div className="space-y-2">
        <Label>Chat Type</Label>
        <Select
          value={chatMode}
          onValueChange={(v) => {
            const mode = v as 'dm_wildcard' | 'specific'
            onChatModeChange(mode)
            if (mode === 'dm_wildcard') {
              onSourceIdChange('*')
            } else {
              onSourceIdChange('')
            }
          }}
          disabled={disabled}
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="dm_wildcard">All Direct Messages</SelectItem>
            <SelectItem value="specific">Specific Chat</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {chatMode === 'dm_wildcard'
            ? 'Each user who DMs the bot gets their own persistent thread and task.'
            : 'Route messages from a specific group or channel.'}
        </p>
      </div>
      {chatMode === 'specific' && (
        <div className="space-y-2">
          <Label>Chat ID</Label>
          <Input
            value={sourceId}
            onChange={(e) => onSourceIdChange(e.target.value)}
            placeholder="-1001234567890 or @username"
            disabled={disabled}
          />
          <p className="text-xs text-muted-foreground">
            The Telegram chat ID. For groups, use the numeric ID (starts with -100). For public
            channels/groups, you can use @username.
          </p>
        </div>
      )}
      <div className="space-y-2">
        <Label>
          Secret Token <span className="text-muted-foreground">(optional)</span>
        </Label>
        <Input
          value={secretToken}
          onChange={(e) => onSecretTokenChange(e.target.value)}
          placeholder="Webhook secret token"
        />
        <p className="text-xs text-muted-foreground">
          If set, Telegram will send this token in the X-Telegram-Bot-Api-Secret-Token header for
          webhook verification.
        </p>
      </div>
      <div className="space-y-2">
        <Label>
          Bot Username <span className="text-muted-foreground">(optional)</span>
        </Label>
        <Input
          value={botUsername}
          onChange={(e) => onBotUsernameChange(e.target.value)}
          placeholder="@mybot"
        />
      </div>
    </>
  )
}

export function ChannelConfigFormDialog({
  open,
  onOpenChange,
  editConfig,
  enabledProviders = ['slack'],
}: ChannelConfigFormDialogProps) {
  const defaultProvider = enabledProviders[0] ?? 'slack'
  const [form, setForm] = useState<FormState>(() => makeInitialForm(defaultProvider))

  const isEditing = !!editConfig
  const createConfig = useCreateChannelConfig()
  const updateConfig = useUpdateChannelConfig()
  const isPending = createConfig.isPending || updateConfig.isPending

  // Slack channels for dropdown
  const {
    data: slackChannels,
    isLoading: slackLoading,
    error: slackError,
  } = useSlackChannels({ enabled: open && form.provider === 'slack' })

  // Repositories for dropdown
  const { data: repositories, isLoading: reposLoading } = useRepositoriesList({
    enabled: open,
  })

  // Non-template repositories
  const nonTemplateRepos = repositories?.filter((r) => r.kind !== 'template') ?? []

  // Pre-fill form when editing config changes
  useEffect(() => {
    if (editConfig) {
      setForm(formStateFromConfig(editConfig))
    } else {
      setForm(makeInitialForm(defaultProvider))
    }
  }, [editConfig, defaultProvider])

  const handleSubmit = () => {
    if (!form.sourceId) {
      toast.error('Please select a channel')
      return
    }
    if (!form.repositoryId) {
      toast.error('Please select a repository')
      return
    }

    const { sourceName, providerConfig } = buildProviderPayload(form, slackChannels)

    if (isEditing && editConfig) {
      updateConfig.mutate(
        {
          id: editConfig.id,
          request: {
            sourceName,
            routingMode: form.routingMode,
            promptTemplate: form.promptTemplate || undefined,
            active: form.active,
            providerConfig,
          },
        },
        {
          onSuccess: () => {
            toast.success('Channel config updated')
            onOpenChange(false)
          },
          onError: (error) => {
            toast.error('Failed to update channel config', {
              description: getErrorMessage(error),
            })
          },
        },
      )
    } else {
      createConfig.mutate(
        {
          provider: form.provider,
          sourceId: form.sourceId,
          sourceName,
          repositoryId: form.repositoryId,
          routingMode: form.routingMode,
          promptTemplate: form.promptTemplate || undefined,
          providerConfig,
        },
        {
          onSuccess: () => {
            toast.success('Channel config created')
            onOpenChange(false)
          },
          onError: (error) => {
            toast.error('Failed to create channel config', {
              description: getErrorMessage(error),
            })
          },
        },
      )
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Edit Channel Config' : 'Add Channel Config'}</DialogTitle>
          <DialogDescription>
            {isEditing
              ? 'Update the channel routing configuration.'
              : 'Route messages from an external channel to a repository.'}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {/* Provider */}
          <div className="space-y-2">
            <Label>Provider</Label>
            <Select
              value={form.provider}
              onValueChange={(v) =>
                setForm((prev) => ({
                  ...prev,
                  provider: v,
                  sourceId: '',
                  secretToken: '',
                  botUsername: '',
                }))
              }
              disabled={isEditing}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select provider" />
              </SelectTrigger>
              <SelectContent>
                {enabledProviders.map((p) => (
                  <SelectItem key={p} value={p}>
                    {PROVIDER_LABELS[p] ?? p}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Provider-specific fields */}
          {form.provider === 'slack' && (
            <SlackChannelField
              sourceId={form.sourceId}
              onChange={(v) => setForm((prev) => ({ ...prev, sourceId: v }))}
              disabled={isEditing}
              slackChannels={slackChannels}
              slackLoading={slackLoading}
              slackError={!!slackError}
            />
          )}
          {form.provider === 'telegram' && (
            <TelegramFields
              sourceId={form.sourceId}
              secretToken={form.secretToken}
              botUsername={form.botUsername}
              chatMode={form.chatMode}
              onSourceIdChange={(v) => setForm((prev) => ({ ...prev, sourceId: v }))}
              onSecretTokenChange={(v) => setForm((prev) => ({ ...prev, secretToken: v }))}
              onBotUsernameChange={(v) => setForm((prev) => ({ ...prev, botUsername: v }))}
              onChatModeChange={(v) => setForm((prev) => ({ ...prev, chatMode: v }))}
              disabled={isEditing}
            />
          )}

          {/* Repository */}
          <div className="space-y-2">
            <Label>Repository</Label>
            <Select
              value={form.repositoryId}
              onValueChange={(v) => setForm((prev) => ({ ...prev, repositoryId: v }))}
              disabled={isEditing || reposLoading}
            >
              <SelectTrigger>
                {reposLoading ? (
                  <div className="flex items-center gap-2">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    <span className="text-muted-foreground">Loading repositories...</span>
                  </div>
                ) : (
                  <SelectValue placeholder="Select a repository" />
                )}
              </SelectTrigger>
              <SelectContent>
                {nonTemplateRepos.map((repo) => (
                  <SelectItem key={repo.id} value={repo.id}>
                    {repo.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Routing Mode */}
          <div className="space-y-2">
            <Label>Routing Mode</Label>
            <Select
              value={form.routingMode}
              onValueChange={(v) => setForm((prev) => ({ ...prev, routingMode: v }))}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="thread_based">Thread Based</SelectItem>
                <SelectItem value="per_message">Per Message</SelectItem>
                <SelectItem value="ongoing">Ongoing</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {ROUTING_MODE_DESCRIPTIONS[form.routingMode]}
            </p>
          </div>

          {/* Trigger Mode (Slack-specific) */}
          {form.provider === 'slack' && (
            <div className="space-y-2">
              <Label>Trigger</Label>
              <Select
                value={form.triggerMode}
                onValueChange={(v) => setForm((prev) => ({ ...prev, triggerMode: v }))}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="mention_only">Mention Only</SelectItem>
                  <SelectItem value="all_messages">All Messages</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {form.triggerMode === 'mention_only'
                  ? 'Only respond when the bot is @mentioned.'
                  : 'Respond to every message in the channel.'}
              </p>
            </div>
          )}

          {/* Prompt Template */}
          <div className="space-y-2">
            <Label>
              Prompt Template <span className="text-muted-foreground">(optional)</span>
            </Label>
            <Textarea
              value={form.promptTemplate}
              onChange={(e) => setForm((prev) => ({ ...prev, promptTemplate: e.target.value }))}
              placeholder="Custom prompt wrapping the message. Use {message} as placeholder."
              rows={3}
            />
          </div>

          {/* Active Toggle */}
          <div className="flex items-center justify-between">
            <div>
              <Label>Active</Label>
              <p className="text-xs text-muted-foreground">
                Disable to pause routing without deleting the config.
              </p>
            </div>
            <Switch
              checked={form.active}
              onCheckedChange={(v) => setForm((prev) => ({ ...prev, active: v }))}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={isPending || !form.sourceId || !form.repositoryId}
          >
            {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isEditing ? 'Save Changes' : 'Create'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

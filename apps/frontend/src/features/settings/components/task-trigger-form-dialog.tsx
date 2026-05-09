import {
  type CreateTaskTriggerRequest,
  IntegrationProvider,
  type ProviderConnectionResponse,
  type TaskRouteResponse,
  type TaskTriggerResponse,
} from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useCreateProviderConnection,
  useCreateTaskTrigger,
  useUpdateTaskTrigger,
} from '@/features/settings/hooks/use-integrations'
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

type JsonObject = Record<string, unknown>

interface TaskTriggerFormDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  trigger?: TaskTriggerResponse | null
  providers: string[]
  routes: TaskRouteResponse[]
  connections: ProviderConnectionResponse[]
  getRouteLabel: (routeId: string) => string
}

interface TaskTriggerFormState {
  provider: string
  connectionMode: 'existing' | 'new'
  connectionId: string
  newExternalAccountId: string
  newSourceName: string
  newConnectionKind: string
  newWebhookSecret: string
  newTransportConfigJson: string
  sourceMatcher: string
  sourceName: string
  routeId: string
  active: boolean
  slackTrigger: string
  eventFilterJson: string
  triggerConfigJson: string
}

const NEW_CONNECTION_VALUE = '__new_connection__'

const PROVIDER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

const SLACK_TRIGGER_LABELS: Record<string, string> = {
  mention_only: 'Mention Only',
  all_messages: 'All Messages',
}

const INTEGRATION_PROVIDER_VALUES = Object.values(IntegrationProvider)

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function toIntegrationProvider(provider: string): IntegrationProvider | null {
  return INTEGRATION_PROVIDER_VALUES.includes(provider as IntegrationProvider)
    ? (provider as IntegrationProvider)
    : null
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function parseJsonObject(value: string, label: string): JsonObject | null {
  const trimmed = value.trim()
  if (!trimmed) {
    return null
  }

  const parsed: unknown = JSON.parse(trimmed)
  if (parsed === null) {
    return null
  }
  if (!isJsonObject(parsed)) {
    throw new Error(`${label} must be a JSON object`)
  }
  return parsed
}

function stringifyJsonObject(value: JsonObject | null | undefined): string {
  return JSON.stringify(value ?? {}, null, 2)
}

function getStringValue(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function defaultConnectionKind(provider: string): string {
  if (provider === 'webhook') {
    return 'webhook'
  }
  if (provider === 'github') {
    return 'org_webhook'
  }
  return 'bot_api'
}

function defaultProvider(providers: string[]): string {
  return providers[0] ?? IntegrationProvider.slack
}

function firstConnectionIdForProvider(
  provider: string,
  connections: ProviderConnectionResponse[],
): string {
  return connections.find((connection) => connection.provider === provider)?.id ?? ''
}

function defaultConnectionMode(provider: string, connections: ProviderConnectionResponse[]) {
  return firstConnectionIdForProvider(provider, connections) ? 'existing' : 'new'
}

function getSlackTrigger(triggerConfig: JsonObject | null | undefined): string {
  return getStringValue(triggerConfig?.trigger) ?? 'mention_only'
}

function makeInitialForm(
  trigger: TaskTriggerResponse | null | undefined,
  providers: string[],
  routes: TaskRouteResponse[],
  connections: ProviderConnectionResponse[],
): TaskTriggerFormState {
  const provider = trigger?.provider ?? defaultProvider(providers)
  const connectionId = trigger?.connectionId ?? firstConnectionIdForProvider(provider, connections)
  const triggerConfig = trigger?.triggerConfig ?? {}

  return {
    provider,
    connectionMode: trigger ? 'existing' : defaultConnectionMode(provider, connections),
    connectionId,
    newExternalAccountId: '',
    newSourceName: '',
    newConnectionKind: defaultConnectionKind(provider),
    newWebhookSecret: '',
    newTransportConfigJson: '{}',
    sourceMatcher: trigger?.sourceMatcher ?? '',
    sourceName: trigger?.sourceName ?? '',
    routeId: trigger?.routeId ?? routes[0]?.id ?? '',
    active: trigger?.active ?? true,
    slackTrigger: getSlackTrigger(triggerConfig),
    eventFilterJson: stringifyJsonObject(trigger?.eventFilter ?? {}),
    triggerConfigJson: stringifyJsonObject(triggerConfig),
  }
}

function mergeSlackTriggerConfigText(value: string, slackTrigger: string): string {
  try {
    const parsed = parseJsonObject(value, 'Trigger Config JSON') ?? {}
    return stringifyJsonObject({ ...parsed, trigger: slackTrigger })
  } catch {
    return value
  }
}

function getValidationMessage(form: TaskTriggerFormState, provider: IntegrationProvider | null) {
  if (!provider) {
    return 'Select a provider'
  }
  if (!form.sourceMatcher.trim()) {
    return 'Enter a source matcher'
  }
  if (!form.routeId) {
    return 'Select a task route'
  }
  if (form.connectionMode === 'existing' && !form.connectionId) {
    return 'Select a connection'
  }
  if (form.connectionMode === 'new' && !form.newExternalAccountId.trim()) {
    return 'Enter an external account ID'
  }
  return null
}

function buildTriggerConfig(
  form: TaskTriggerFormState,
  provider: IntegrationProvider,
): JsonObject | null {
  const parsedTriggerConfig = parseJsonObject(form.triggerConfigJson, 'Trigger Config JSON')
  if (provider === IntegrationProvider.slack) {
    return { ...(parsedTriggerConfig ?? {}), trigger: form.slackTrigger }
  }
  return parsedTriggerConfig
}

type TriggerMutableFields = Omit<CreateTaskTriggerRequest, 'provider'>

function buildTriggerFields(
  form: TaskTriggerFormState,
  connectionId: string,
  eventFilter: JsonObject | null,
  triggerConfig: JsonObject | null,
): TriggerMutableFields {
  return {
    connectionId,
    sourceMatcher: form.sourceMatcher.trim(),
    sourceName: form.sourceName.trim() || null,
    routeId: form.routeId,
    active: form.active,
    eventFilter,
    triggerConfig,
    allowDirectSend: false,
    closeTaskOnClose: true,
    notifyOnSuccess: false,
  }
}

export function TaskTriggerFormDialog({
  open,
  onOpenChange,
  trigger,
  providers,
  routes,
  connections,
  getRouteLabel,
}: TaskTriggerFormDialogProps) {
  const [form, setForm] = useState<TaskTriggerFormState>(() =>
    makeInitialForm(trigger, providers, routes, connections),
  )
  const createConnection = useCreateProviderConnection()
  const createTrigger = useCreateTaskTrigger()
  const updateTrigger = useUpdateTaskTrigger()
  const isEditing = !!trigger
  const isPending = createConnection.isPending || createTrigger.isPending || updateTrigger.isPending

  const providerConnections = useMemo(
    () => connections.filter((connection) => connection.provider === form.provider),
    [connections, form.provider],
  )

  useEffect(() => {
    if (open) {
      setForm(makeInitialForm(trigger, providers, routes, connections))
    }
  }, [open, trigger, providers, routes, connections])

  const setProvider = (provider: string) => {
    const firstConnectionId = firstConnectionIdForProvider(provider, connections)
    setForm((prev) => ({
      ...prev,
      provider,
      connectionMode: firstConnectionId ? 'existing' : 'new',
      connectionId: firstConnectionId,
      newConnectionKind: defaultConnectionKind(provider),
      slackTrigger: 'mention_only',
      triggerConfigJson:
        provider === IntegrationProvider.slack ? '{\n  "trigger": "mention_only"\n}' : '{}',
    }))
  }

  const handleSubmit = async () => {
    const provider = toIntegrationProvider(form.provider)
    const validationMessage = getValidationMessage(form, provider)
    if (validationMessage || !provider) {
      toast.error(validationMessage ?? 'Select a provider')
      return
    }

    try {
      const eventFilter = parseJsonObject(form.eventFilterJson, 'Event Filter JSON')
      const triggerConfig = buildTriggerConfig(form, provider)

      let connectionId = form.connectionId
      if (form.connectionMode === 'new') {
        const transportConfig = parseJsonObject(
          form.newTransportConfigJson,
          'Transport Config JSON',
        )
        const connection = await createConnection.mutateAsync({
          provider,
          externalAccountId: form.newExternalAccountId.trim(),
          sourceName: form.newSourceName.trim() || null,
          connectionKind: form.newConnectionKind.trim() || null,
          webhookSecret: form.newWebhookSecret.trim() || null,
          transportConfig,
        })
        connectionId = connection.id
      }

      const triggerFields = buildTriggerFields(form, connectionId, eventFilter, triggerConfig)

      if (isEditing && trigger) {
        await updateTrigger.mutateAsync({ id: trigger.id, request: triggerFields })
        toast.success('Trigger updated')
      } else {
        await createTrigger.mutateAsync({ provider, ...triggerFields })
        toast.success('Trigger created')
      }
      onOpenChange(false)
    } catch (error) {
      toast.error(isEditing ? 'Failed to update trigger' : 'Failed to create trigger', {
        description: getErrorMessage(error),
      })
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[620px]">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Edit Trigger' : 'Add Trigger'}</DialogTitle>
          <DialogDescription>
            {isEditing ? 'Update trigger routing.' : 'Create an integration trigger.'}
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[68vh] space-y-4 overflow-y-auto py-4 pr-1">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>Provider</Label>
              <Select value={form.provider} onValueChange={setProvider} disabled={isEditing}>
                <SelectTrigger>
                  <SelectValue placeholder="Select provider" />
                </SelectTrigger>
                <SelectContent>
                  {providers.map((provider) => (
                    <SelectItem key={provider} value={provider}>
                      {getProviderLabel(provider)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>Connection</Label>
              <Select
                value={
                  form.connectionMode === 'new'
                    ? NEW_CONNECTION_VALUE
                    : form.connectionId || NEW_CONNECTION_VALUE
                }
                onValueChange={(value) => {
                  if (value === NEW_CONNECTION_VALUE) {
                    setForm((prev) => ({ ...prev, connectionMode: 'new', connectionId: '' }))
                  } else {
                    setForm((prev) => ({
                      ...prev,
                      connectionMode: 'existing',
                      connectionId: value,
                    }))
                  }
                }}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select connection" />
                </SelectTrigger>
                <SelectContent>
                  {providerConnections.map((connection) => (
                    <SelectItem key={connection.id} value={connection.id}>
                      {connection.externalAccountId}
                    </SelectItem>
                  ))}
                  <SelectItem value={NEW_CONNECTION_VALUE}>Create New Connection</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          {form.connectionMode === 'new' && (
            <div className="rounded-md border p-3">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label>External Account ID</Label>
                  <Input
                    value={form.newExternalAccountId}
                    onChange={(event) =>
                      setForm((prev) => ({ ...prev, newExternalAccountId: event.target.value }))
                    }
                    placeholder="workspace, org, or account ID"
                  />
                </div>
                <div className="space-y-2">
                  <Label>
                    Name <span className="text-muted-foreground">(optional)</span>
                  </Label>
                  <Input
                    value={form.newSourceName}
                    onChange={(event) =>
                      setForm((prev) => ({ ...prev, newSourceName: event.target.value }))
                    }
                    placeholder="Display name"
                  />
                </div>
                <div className="space-y-2">
                  <Label>Connection Kind</Label>
                  <Input
                    value={form.newConnectionKind}
                    onChange={(event) =>
                      setForm((prev) => ({ ...prev, newConnectionKind: event.target.value }))
                    }
                    placeholder="bot_api"
                  />
                </div>
                <div className="space-y-2">
                  <Label>
                    Webhook Secret <span className="text-muted-foreground">(optional)</span>
                  </Label>
                  <Input
                    type="password"
                    value={form.newWebhookSecret}
                    onChange={(event) =>
                      setForm((prev) => ({ ...prev, newWebhookSecret: event.target.value }))
                    }
                    placeholder="Secret"
                  />
                </div>
              </div>
              <div className="mt-4 space-y-2">
                <Label>
                  Transport Config JSON <span className="text-muted-foreground">(optional)</span>
                </Label>
                <Textarea
                  value={form.newTransportConfigJson}
                  onChange={(event) =>
                    setForm((prev) => ({
                      ...prev,
                      newTransportConfigJson: event.target.value,
                    }))
                  }
                  rows={3}
                />
              </div>
            </div>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>Source Matcher</Label>
              <Input
                value={form.sourceMatcher}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, sourceMatcher: event.target.value }))
                }
                placeholder="C01234567890, owner/repo, or *"
              />
            </div>
            <div className="space-y-2">
              <Label>
                Source Name <span className="text-muted-foreground">(optional)</span>
              </Label>
              <Input
                value={form.sourceName}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, sourceName: event.target.value }))
                }
                placeholder="Human-readable source"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>Task Route</Label>
            <Select
              value={form.routeId}
              onValueChange={(routeId) => setForm((prev) => ({ ...prev, routeId }))}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select route" />
              </SelectTrigger>
              <SelectContent>
                {routes.map((route) => (
                  <SelectItem key={route.id} value={route.id}>
                    {getRouteLabel(route.id)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {form.provider === IntegrationProvider.slack && (
            <div className="space-y-2">
              <Label>Slack Trigger</Label>
              <Select
                value={form.slackTrigger}
                onValueChange={(slackTrigger) =>
                  setForm((prev) => ({
                    ...prev,
                    slackTrigger,
                    triggerConfigJson: mergeSlackTriggerConfigText(
                      prev.triggerConfigJson,
                      slackTrigger,
                    ),
                  }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Object.entries(SLACK_TRIGGER_LABELS).map(([value, label]) => (
                    <SelectItem key={value} value={value}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>Event Filter JSON</Label>
              <Textarea
                value={form.eventFilterJson}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, eventFilterJson: event.target.value }))
                }
                rows={5}
              />
            </div>
            <div className="space-y-2">
              <Label>Trigger Config JSON</Label>
              <Textarea
                value={form.triggerConfigJson}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, triggerConfigJson: event.target.value }))
                }
                rows={5}
              />
            </div>
          </div>

          <div className="flex items-center justify-between rounded-md border p-3">
            <Label>Active</Label>
            <Switch
              checked={form.active}
              onCheckedChange={(active) => setForm((prev) => ({ ...prev, active }))}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={
              isPending ||
              !form.sourceMatcher.trim() ||
              !form.routeId ||
              (form.connectionMode === 'existing' && !form.connectionId) ||
              (form.connectionMode === 'new' && !form.newExternalAccountId.trim())
            }
          >
            {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isEditing ? 'Save Changes' : 'Create'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

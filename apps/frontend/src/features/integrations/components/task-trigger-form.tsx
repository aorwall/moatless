import {
  type CreateTaskTriggerRequest,
  IntegrationProvider,
  type RepositoryResponse,
  type TaskRouteResponse,
  type TaskTriggerResponse,
} from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useCreateTaskTrigger,
  useIntegrationProviders,
  useProviderConnections,
  useTaskRoutes,
  useUpdateTaskTrigger,
} from '@/features/integrations/hooks/use-integrations'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { Button } from '@/lib/components/ui/button'
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

interface TaskTriggerFormProps {
  trigger?: TaskTriggerResponse | null
  onSuccess?: () => void
  onCancel?: () => void
}

interface TaskTriggerFormState {
  provider: string
  connectionId: string
  sourceMatcher: string
  sourceName: string
  routeId: string
  active: boolean
  slackTrigger: string
  eventFilterJson: string
  triggerConfigJson: string
}

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

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
}

const INTEGRATION_PROVIDER_VALUES = Object.values(IntegrationProvider)

function getProviderLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
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

function defaultProvider(providers: string[]): string {
  return providers[0] ?? IntegrationProvider.slack
}

function getSlackTrigger(triggerConfig: JsonObject | null | undefined): string {
  return getStringValue(triggerConfig?.trigger) ?? 'mention_only'
}

function getRouteLabel(route: TaskRouteResponse, repositoriesById: Map<string, string>): string {
  const repositoryName = repositoriesById.get(route.repositoryId) ?? route.repositoryId
  const taskName = route.taskName ? ` - ${route.taskName}` : ''
  return `${repositoryName} - ${getRoutingModeLabel(route.routingMode)}${taskName}`
}

function makeInitialForm(
  trigger: TaskTriggerResponse | null | undefined,
  providers: string[],
  routes: TaskRouteResponse[],
): TaskTriggerFormState {
  const provider = trigger?.provider ?? defaultProvider(providers)
  const triggerConfig = trigger?.triggerConfig ?? {}

  return {
    provider,
    connectionId: trigger?.connectionId ?? '',
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
  eventFilter: JsonObject | null,
  triggerConfig: JsonObject | null,
): TriggerMutableFields {
  return {
    connectionId: form.connectionId,
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

function createRepositoryMap(repositories: RepositoryResponse[] | undefined) {
  return new Map((repositories ?? []).map((repository) => [repository.id, repository.name]))
}

export function TaskTriggerForm({ trigger, onSuccess, onCancel }: TaskTriggerFormProps) {
  const { data: providers } = useIntegrationProviders()
  const { data: connections } = useProviderConnections()
  const { data: routes } = useTaskRoutes()
  const { data: repositories } = useRepositoriesList()

  const providerItems = providers ?? []
  const connectionItems = useMemo(() => connections ?? [], [connections])
  const routeItems = useMemo(() => routes ?? [], [routes])
  const repositoriesById = useMemo(() => createRepositoryMap(repositories), [repositories])

  const [form, setForm] = useState<TaskTriggerFormState>(() =>
    makeInitialForm(trigger, providerItems, routeItems),
  )
  const createTrigger = useCreateTaskTrigger()
  const updateTrigger = useUpdateTaskTrigger()
  const isEditing = !!trigger
  const isPending = createTrigger.isPending || updateTrigger.isPending

  const providerConnections = useMemo(
    () => connectionItems.filter((connection) => connection.provider === form.provider),
    [connectionItems, form.provider],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-init only on trigger identity change
  useEffect(() => {
    setForm(makeInitialForm(trigger, providerItems, routeItems))
  }, [trigger?.id])

  const setProvider = (provider: string) => {
    setForm((prev) => ({
      ...prev,
      provider,
      connectionId: '',
      slackTrigger: 'mention_only',
      triggerConfigJson:
        provider === IntegrationProvider.slack ? '{\n  "trigger": "mention_only"\n}' : '{}',
    }))
  }

  const handleSubmit = async () => {
    const provider = toIntegrationProvider(form.provider)
    if (!provider) {
      toast.error('Select a provider')
      return
    }
    if (!form.sourceMatcher.trim()) {
      toast.error('Enter a source matcher')
      return
    }
    if (!form.routeId) {
      toast.error('Select a task route')
      return
    }
    if (!form.connectionId) {
      toast.error('Select a connection')
      return
    }

    try {
      const eventFilter = parseJsonObject(form.eventFilterJson, 'Event Filter JSON')
      const triggerConfig = buildTriggerConfig(form, provider)
      const triggerFields = buildTriggerFields(form, eventFilter, triggerConfig)

      if (isEditing && trigger) {
        await updateTrigger.mutateAsync({ id: trigger.id, request: triggerFields })
        toast.success('Trigger updated')
      } else {
        await createTrigger.mutateAsync({ provider, ...triggerFields })
        toast.success('Trigger created')
      }
      onSuccess?.()
    } catch (error) {
      toast.error(isEditing ? 'Failed to update trigger' : 'Failed to create trigger', {
        description: getErrorMessage(error),
      })
    }
  }

  const noConnectionsForProvider = providerConnections.length === 0
  const submitDisabled =
    isPending ||
    !form.sourceMatcher.trim() ||
    !form.routeId ||
    !form.connectionId ||
    noConnectionsForProvider

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label>Provider</Label>
          <Select value={form.provider} onValueChange={setProvider} disabled={isEditing}>
            <SelectTrigger>
              <SelectValue placeholder="Select provider" />
            </SelectTrigger>
            <SelectContent>
              {providerItems.map((provider) => (
                <SelectItem key={provider} value={provider}>
                  {getProviderLabel(provider)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-2">
          <Label>Connection</Label>
          {noConnectionsForProvider ? (
            <div className="rounded-md border px-3 py-2 text-sm text-muted-foreground">
              No connections for {getProviderLabel(form.provider)}.{' '}
              <Link
                to="/admin/integrations/connections"
                className="text-primary underline hover:no-underline"
              >
                Create a connection
              </Link>
            </div>
          ) : (
            <Select
              value={form.connectionId}
              onValueChange={(connectionId) => setForm((prev) => ({ ...prev, connectionId }))}
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
              </SelectContent>
            </Select>
          )}
        </div>
      </div>

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
            onChange={(event) => setForm((prev) => ({ ...prev, sourceName: event.target.value }))}
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
            {routeItems.map((route) => (
              <SelectItem key={route.id} value={route.id}>
                {getRouteLabel(route, repositoriesById)}
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

      <div className="flex justify-end gap-2 pt-2">
        {onCancel && (
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
        )}
        <Button onClick={handleSubmit} disabled={submitDisabled}>
          {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          {isEditing ? 'Save Changes' : 'Create'}
        </Button>
      </div>
    </div>
  )
}

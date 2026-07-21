import {
  AdapterKind,
  type CreateTriggerRequest,
  RoutingMode,
  TriggerKind,
  type TriggerResponse,
  type UpdateTriggerRequest,
} from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useCreateTrigger,
  useIntegrationAdapters,
  useIntegrationConnections,
  useUpdateTrigger,
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
type TriggerSource = TriggerResponse['source']

interface TriggerFormProps {
  trigger?: TriggerResponse | null
  onSuccess?: () => void
  onCancel?: () => void
}

interface TriggerFormState {
  kind: TriggerKind
  name: string
  active: boolean
  repositoryId: string
  routingMode: RoutingMode
  prompt: string
  taskName: string
  tagIdsText: string
  runAsUserId: string
  agentType: string
  adapter: AdapterKind
  connectionId: string
  sourceMatcher: string
  sourceName: string
  eventFilterJson: string
  adapterConfigJson: string
  allowDirectSend: boolean
  closeTaskOnClose: boolean
  notifyOnSuccess: boolean
  cronExpression: string
  timezone: string
  messageTemplate: string
}

const ADAPTER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
}

function getAdapterLabel(adapter: string): string {
  return ADAPTER_LABELS[adapter] ?? adapter
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
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

function isAdapterEventSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { adapterKind: AdapterKind }> {
  return 'adapterKind' in source
}

function isScheduleSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { cronExpression: string }> {
  return 'cronExpression' in source
}

function parseTags(value: string): string[] {
  return value
    .split(',')
    .map((tag) => tag.trim())
    .filter(Boolean)
}

function defaultAdapter(adapters: string[]): AdapterKind {
  const first = adapters[0]
  if (Object.values(AdapterKind).includes(first as AdapterKind)) {
    return first as AdapterKind
  }
  return AdapterKind.slack
}

function makeRuleFields(rule: TriggerResponse['rule'] | undefined) {
  return {
    repositoryId: rule?.repositoryId ?? '',
    routingMode: rule?.routingMode ?? RoutingMode.by_subject,
    prompt: rule?.prompt ?? '',
    taskName: rule?.taskName ?? '',
    tagIdsText: rule?.tagIds.join(', ') ?? '',
    runAsUserId: rule?.runAsUserId ?? '',
    agentType: rule?.agentType ?? 'claude-code',
  }
}

function makeAdapterFields(source: TriggerSource | undefined, adapters: string[]) {
  const adapterSource = source && isAdapterEventSource(source) ? source : null

  return {
    adapter: adapterSource?.adapterKind ?? defaultAdapter(adapters),
    connectionId: adapterSource?.connectionId ?? '',
    sourceMatcher: adapterSource?.sourceMatcher ?? '',
    sourceName: adapterSource?.sourceName ?? '',
    eventFilterJson: stringifyJsonObject(adapterSource?.eventFilter ?? {}),
    adapterConfigJson: stringifyJsonObject(adapterSource?.adapterConfig ?? {}),
    allowDirectSend: adapterSource?.allowDirectSend ?? false,
    closeTaskOnClose: adapterSource?.closeTaskOnClose ?? true,
    notifyOnSuccess: adapterSource?.notifyOnSuccess ?? false,
  }
}

function makeScheduleFields(source: TriggerSource | undefined) {
  const scheduleSource = source && isScheduleSource(source) ? source : null

  return {
    cronExpression: scheduleSource?.cronExpression ?? '',
    timezone: scheduleSource?.timezone ?? 'UTC',
    messageTemplate: scheduleSource?.messageTemplate ?? '',
  }
}

function makeInitialForm(
  trigger: TriggerResponse | null | undefined,
  adapters: string[],
): TriggerFormState {
  return {
    kind: trigger?.kind ?? TriggerKind.adapter_event,
    name: trigger?.name ?? '',
    active: trigger?.active ?? true,
    ...makeRuleFields(trigger?.rule),
    ...makeAdapterFields(trigger?.source, adapters),
    ...makeScheduleFields(trigger?.source),
  }
}

function validateForm(form: TriggerFormState): string | null {
  if (!form.repositoryId) {
    return 'Select a repository'
  }
  if (form.kind === TriggerKind.adapter_event) {
    if (!form.connectionId) {
      return 'Select a connection'
    }
    if (!form.sourceMatcher.trim()) {
      return 'Enter a source matcher'
    }
  }
  if (form.kind === TriggerKind.schedule) {
    if (!form.cronExpression.trim()) {
      return 'Enter a cron expression'
    }
    if (!form.messageTemplate.trim()) {
      return 'Enter a message template'
    }
  }
  return null
}

function buildRule(form: TriggerFormState): CreateTriggerRequest['rule'] {
  return {
    repositoryId: form.repositoryId,
    routingMode: form.routingMode,
    prompt: form.prompt.trim() || null,
    taskName: form.taskName.trim() || null,
    tagIds: parseTags(form.tagIdsText),
    runAsUserId: form.runAsUserId.trim() || null,
    agentType: form.agentType.trim() || 'claude-code',
  }
}

function buildAdapterUpdateSource(
  form: TriggerFormState,
): NonNullable<UpdateTriggerRequest['source']> {
  return {
    connectionId: form.connectionId,
    sourceMatcher: form.sourceMatcher.trim(),
    sourceName: form.sourceName.trim() || null,
    eventFilter: parseJsonObject(form.eventFilterJson, 'Event Filter JSON'),
    adapterConfig: parseJsonObject(form.adapterConfigJson, 'Adapter Config JSON'),
    allowDirectSend: form.allowDirectSend,
    closeTaskOnClose: form.closeTaskOnClose,
    notifyOnSuccess: form.notifyOnSuccess,
  }
}

function buildAdapterCreateSource(form: TriggerFormState): CreateTriggerRequest['source'] {
  return {
    adapterKind: form.adapter,
    ...buildAdapterUpdateSource(form),
  }
}

function buildScheduleUpdateSource(
  form: TriggerFormState,
): NonNullable<UpdateTriggerRequest['source']> {
  return {
    cronExpression: form.cronExpression.trim(),
    timezone: form.timezone.trim() || 'UTC',
    messageTemplate: form.messageTemplate,
  }
}

function buildUpdateSource(form: TriggerFormState): NonNullable<UpdateTriggerRequest['source']> {
  return form.kind === TriggerKind.adapter_event
    ? buildAdapterUpdateSource(form)
    : buildScheduleUpdateSource(form)
}

function buildCreateSource(form: TriggerFormState): CreateTriggerRequest['source'] {
  return form.kind === TriggerKind.adapter_event
    ? buildAdapterCreateSource(form)
    : {
        cronExpression: form.cronExpression.trim(),
        timezone: form.timezone.trim() || 'UTC',
        messageTemplate: form.messageTemplate,
      }
}

export function TaskTriggerForm(props: TriggerFormProps) {
  const { data: adapters, isLoading, error } = useIntegrationAdapters()

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!adapters) {
    return (
      <div className="py-8 text-center text-sm text-destructive">
        Failed to load integration adapters: {getErrorMessage(error)}
      </div>
    )
  }

  return (
    <TaskTriggerFormFields
      key={`${props.trigger?.id ?? 'new'}:${adapters.join(',')}`}
      {...props}
      adapters={adapters}
    />
  )
}

function TaskTriggerFormFields({
  trigger,
  onSuccess,
  onCancel,
  adapters,
}: TriggerFormProps & { adapters: string[] }) {
  const { data: connections } = useIntegrationConnections()
  const { data: repositories } = useRepositoriesList()
  const adapterItems = adapters
  const isEditing = !!trigger
  const createTrigger = useCreateTrigger()
  const updateTrigger = useUpdateTrigger()
  const isPending = createTrigger.isPending || updateTrigger.isPending

  const [form, setForm] = useState<TriggerFormState>(() => makeInitialForm(trigger, adapterItems))

  const connectionItems = (connections ?? []).filter(
    (connection) => connection.adapterKind === form.adapter,
  )

  const handleSubmit = async () => {
    const validationError = validateForm(form)
    if (validationError) {
      toast.error(validationError)
      return
    }

    try {
      const name = form.name.trim() || null

      if (isEditing && trigger) {
        await updateTrigger.mutateAsync({
          id: trigger.id,
          request: {
            name,
            active: form.active,
            rule: buildRule(form),
            source: buildUpdateSource(form),
          },
        })
        toast.success('Trigger updated')
      } else {
        await createTrigger.mutateAsync({
          name,
          kind: form.kind,
          active: form.active,
          rule: buildRule(form),
          source: buildCreateSource(form),
        })
        toast.success('Trigger created')
      }
      onSuccess?.()
    } catch (error) {
      toast.error(isEditing ? 'Failed to update trigger' : 'Failed to create trigger', {
        description: getErrorMessage(error),
      })
    }
  }

  const submitDisabled =
    isPending ||
    !form.repositoryId ||
    (form.kind === TriggerKind.adapter_event &&
      (!(form.connectionId && form.sourceMatcher.trim()) || connectionItems.length === 0)) ||
    (form.kind === TriggerKind.schedule &&
      !(form.cronExpression.trim() && form.messageTemplate.trim()))

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label>Kind</Label>
          <Select
            value={form.kind}
            onValueChange={(kind) => setForm((prev) => ({ ...prev, kind: kind as TriggerKind }))}
            disabled={isEditing}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={TriggerKind.adapter_event}>Adapter Event</SelectItem>
              <SelectItem value={TriggerKind.schedule}>Schedule</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label>Name</Label>
          <Input
            value={form.name}
            onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
            placeholder="Optional display name"
          />
        </div>
      </div>

      {form.kind === TriggerKind.adapter_event ? (
        <>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>Adapter</Label>
              <Select
                value={form.adapter}
                onValueChange={(adapter) =>
                  setForm((prev) => ({
                    ...prev,
                    adapter: adapter as AdapterKind,
                    connectionId: '',
                  }))
                }
                disabled={isEditing}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {adapterItems.map((adapter) => (
                    <SelectItem key={adapter} value={adapter}>
                      {getAdapterLabel(adapter)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Connection</Label>
              {connectionItems.length === 0 ? (
                <div className="rounded-md border px-3 py-2 text-sm text-muted-foreground">
                  No connections for {getAdapterLabel(form.adapter)}.{' '}
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
                    {connectionItems.map((connection) => (
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
              <Label>Source Name</Label>
              <Input
                value={form.sourceName}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, sourceName: event.target.value }))
                }
                placeholder="Optional source label"
              />
            </div>
          </div>
        </>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-2">
            <Label>Cron Expression</Label>
            <Input
              value={form.cronExpression}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, cronExpression: event.target.value }))
              }
              placeholder="0 0 * * * *"
            />
          </div>
          <div className="space-y-2">
            <Label>Timezone</Label>
            <Input
              value={form.timezone}
              onChange={(event) => setForm((prev) => ({ ...prev, timezone: event.target.value }))}
              placeholder="UTC"
            />
          </div>
          <div className="space-y-2 sm:col-span-2">
            <Label>Message Template</Label>
            <Textarea
              value={form.messageTemplate}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, messageTemplate: event.target.value }))
              }
              rows={4}
            />
          </div>
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label>Repository</Label>
          <Select
            value={form.repositoryId}
            onValueChange={(repositoryId) => setForm((prev) => ({ ...prev, repositoryId }))}
          >
            <SelectTrigger>
              <SelectValue placeholder="Select repository" />
            </SelectTrigger>
            <SelectContent>
              {(repositories ?? []).map((repository) => (
                <SelectItem key={repository.id} value={repository.id}>
                  {repository.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label>Routing Mode</Label>
          <Select
            value={form.routingMode}
            onValueChange={(routingMode) =>
              setForm((prev) => ({ ...prev, routingMode: routingMode as RoutingMode }))
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {Object.values(RoutingMode).map((mode) => (
                <SelectItem key={mode} value={mode}>
                  {getRoutingModeLabel(mode)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label>Task Name</Label>
          <Input
            value={form.taskName}
            onChange={(event) => setForm((prev) => ({ ...prev, taskName: event.target.value }))}
            placeholder="Optional task title"
          />
        </div>
        <div className="space-y-2">
          <Label>Run As User ID</Label>
          <Input
            value={form.runAsUserId}
            onChange={(event) => setForm((prev) => ({ ...prev, runAsUserId: event.target.value }))}
            placeholder="Optional user ID"
          />
        </div>
        <div className="space-y-2">
          <Label>Agent Type</Label>
          <Input
            value={form.agentType}
            onChange={(event) => setForm((prev) => ({ ...prev, agentType: event.target.value }))}
          />
        </div>
        <div className="space-y-2">
          <Label>Tag IDs</Label>
          <Input
            value={form.tagIdsText}
            onChange={(event) => setForm((prev) => ({ ...prev, tagIdsText: event.target.value }))}
            placeholder="tag-id-1, tag-id-2"
          />
        </div>
      </div>

      <div className="space-y-2">
        <Label>Prompt</Label>
        <Textarea
          value={form.prompt}
          onChange={(event) => setForm((prev) => ({ ...prev, prompt: event.target.value }))}
          rows={4}
        />
      </div>

      {form.kind === TriggerKind.adapter_event && (
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
            <Label>Adapter Config JSON</Label>
            <Textarea
              value={form.adapterConfigJson}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, adapterConfigJson: event.target.value }))
              }
              rows={5}
            />
          </div>
        </div>
      )}

      <div className="grid gap-3 rounded-md border p-3 sm:grid-cols-2">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm font-medium">Active</span>
          <Switch
            aria-label="Active"
            checked={form.active}
            onCheckedChange={(active) => setForm((prev) => ({ ...prev, active }))}
          />
        </div>
        {form.kind === TriggerKind.adapter_event && (
          <>
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Direct Send</span>
              <Switch
                aria-label="Direct send"
                checked={form.allowDirectSend}
                onCheckedChange={(allowDirectSend) =>
                  setForm((prev) => ({ ...prev, allowDirectSend }))
                }
              />
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Close Task On Close</span>
              <Switch
                aria-label="Close task on close"
                checked={form.closeTaskOnClose}
                onCheckedChange={(closeTaskOnClose) =>
                  setForm((prev) => ({ ...prev, closeTaskOnClose }))
                }
              />
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Notify On Success</span>
              <Switch
                aria-label="Notify on success"
                checked={form.notifyOnSuccess}
                onCheckedChange={(notifyOnSuccess) =>
                  setForm((prev) => ({ ...prev, notifyOnSuccess }))
                }
              />
            </div>
          </>
        )}
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

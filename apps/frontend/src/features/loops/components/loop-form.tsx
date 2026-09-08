import {
  AdapterKind,
  type CreateLoopRequest,
  type Loop,
  LoopKind,
  RoutingMode,
  type UpdateLoopRequest,
} from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useIntegrationAdapters,
  useIntegrationConnections,
} from '@/features/integrations/hooks/use-integrations'
import { useCreateLoop, useUpdateLoop } from '@/features/loops/hooks/use-loops'
import {
  getAdapterLabel,
  isAdapterEventSource,
  isScheduleSource,
  type LoopSource,
} from '@/features/loops/utils/loop-display'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useWorkspacesList } from '@/features/workspaces/hooks/use-workspaces'
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

interface LoopFormProps {
  loop?: Loop | null
  onSuccess?: () => void
  onCancel?: () => void
}

interface LoopFormState {
  kind: LoopKind
  name: string
  repositoryId: string
  workspaceId: string
  routingMode: RoutingMode
  prompt: string
  taskName: string
  tagIdsText: string
  skillsText: string
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

/** Sentinel for "no workspace named": a `Select` item cannot carry an empty value. */
const ANY_WORKSPACE = '__any__'

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
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

/** Split a comma-separated list field into trimmed, non-empty entries. */
function parseList(value: string): string[] {
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

function makeConfigFields(config: Loop['config'] | undefined) {
  return {
    repositoryId: config?.repositoryId ?? '',
    workspaceId: config?.workspaceId ?? '',
    routingMode: config?.routingMode ?? RoutingMode.by_subject,
    prompt: config?.prompt ?? '',
    taskName: config?.taskName ?? '',
    tagIdsText: config?.tagIds.join(', ') ?? '',
    skillsText: config?.skills.join(', ') ?? '',
    agentType: config?.agentType ?? 'claude-code',
  }
}

function makeAdapterFields(source: LoopSource | undefined, adapters: string[]) {
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

function makeScheduleFields(source: LoopSource | undefined) {
  const scheduleSource = source && isScheduleSource(source) ? source : null

  return {
    cronExpression: scheduleSource?.cronExpression ?? '',
    timezone: scheduleSource?.timezone ?? 'UTC',
    messageTemplate: scheduleSource?.messageTemplate ?? '',
  }
}

function makeInitialForm(loop: Loop | null | undefined, adapters: string[]): LoopFormState {
  return {
    kind: loop?.kind ?? LoopKind.adapter_event,
    name: loop?.name ?? '',
    ...makeConfigFields(loop?.config),
    ...makeAdapterFields(loop?.source, adapters),
    ...makeScheduleFields(loop?.source),
  }
}

function validateForm(form: LoopFormState): string | null {
  if (!form.name.trim()) {
    return 'Enter a name'
  }
  if (!form.repositoryId) {
    return 'Select a repository'
  }
  if (form.kind === LoopKind.adapter_event) {
    if (!form.connectionId) {
      return 'Select a connection'
    }
    if (!form.sourceMatcher.trim()) {
      return 'Enter a source matcher'
    }
  }
  if (form.kind === LoopKind.schedule) {
    if (!form.cronExpression.trim()) {
      return 'Enter a cron expression'
    }
    if (!form.messageTemplate.trim()) {
      return 'Enter a message template'
    }
  }
  return null
}

function buildUpdateConfig(form: LoopFormState): NonNullable<UpdateLoopRequest['config']> {
  return {
    repositoryId: form.repositoryId,
    workspaceId: form.workspaceId || null,
    routingMode: form.routingMode,
    prompt: form.prompt.trim() || null,
    taskName: form.taskName.trim() || null,
    tagIds: parseList(form.tagIdsText),
    skills: parseList(form.skillsText),
    agentType: form.agentType.trim() || 'claude-code',
  }
}

function buildCreateConfig(form: LoopFormState): CreateLoopRequest['config'] {
  return {
    repositoryId: form.repositoryId,
    workspaceId: form.workspaceId || null,
    routingMode: form.routingMode,
    prompt: form.prompt.trim() || null,
    taskName: form.taskName.trim() || null,
    tagIds: parseList(form.tagIdsText),
    skills: parseList(form.skillsText),
    runAsUserId: null,
    agentType: form.agentType.trim() || 'claude-code',
  }
}

function buildAdapterUpdateSource(form: LoopFormState): NonNullable<UpdateLoopRequest['source']> {
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

function buildAdapterCreateSource(form: LoopFormState): CreateLoopRequest['source'] {
  return {
    adapterKind: form.adapter,
    ...buildAdapterUpdateSource(form),
  }
}

function buildScheduleUpdateSource(form: LoopFormState): NonNullable<UpdateLoopRequest['source']> {
  return {
    cronExpression: form.cronExpression.trim(),
    timezone: form.timezone.trim() || 'UTC',
    messageTemplate: form.messageTemplate,
  }
}

function buildUpdateSource(form: LoopFormState): NonNullable<UpdateLoopRequest['source']> {
  return form.kind === LoopKind.adapter_event
    ? buildAdapterUpdateSource(form)
    : buildScheduleUpdateSource(form)
}

function buildCreateSource(form: LoopFormState): CreateLoopRequest['source'] {
  return form.kind === LoopKind.adapter_event
    ? buildAdapterCreateSource(form)
    : {
        cronExpression: form.cronExpression.trim(),
        timezone: form.timezone.trim() || 'UTC',
        messageTemplate: form.messageTemplate,
      }
}

export function LoopForm(props: LoopFormProps) {
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
    <LoopFormFields
      key={`${props.loop?.id ?? 'new'}:${adapters.join(',')}`}
      {...props}
      adapters={adapters}
    />
  )
}

function LoopFormFields({
  loop,
  onSuccess,
  onCancel,
  adapters,
}: LoopFormProps & { adapters: string[] }) {
  const { data: connections } = useIntegrationConnections()
  const { data: repositories } = useRepositoriesList()
  const { data: workspaces } = useWorkspacesList()
  const adapterItems = adapters
  const isEditing = !!loop
  const createLoop = useCreateLoop()
  const updateLoop = useUpdateLoop()
  const isPending = createLoop.isPending || updateLoop.isPending

  const [form, setForm] = useState<LoopFormState>(() => makeInitialForm(loop, adapterItems))

  const connectionItems = (connections ?? []).filter(
    (connection) => connection.adapterKind === form.adapter,
  )
  const workspaceItems = (workspaces ?? []).filter((workspace) =>
    workspace.repos.some((repo) => repo.repositoryId === form.repositoryId),
  )

  const handleSubmit = async () => {
    const validationError = validateForm(form)
    if (validationError) {
      toast.error(validationError)
      return
    }

    try {
      const name = form.name.trim()

      if (isEditing && loop) {
        await updateLoop.mutateAsync({
          id: loop.id,
          request: {
            name,
            config: buildUpdateConfig(form),
            source: buildUpdateSource(form),
          },
        })
        toast.success('Loop updated')
      } else {
        await createLoop.mutateAsync({
          name,
          kind: form.kind,
          active: false,
          config: buildCreateConfig(form),
          source: buildCreateSource(form),
        })
        toast.success('Loop created')
      }
      onSuccess?.()
    } catch (error) {
      toast.error(isEditing ? 'Failed to update loop' : 'Failed to create loop', {
        description: getErrorMessage(error),
      })
    }
  }

  const submitDisabled =
    isPending ||
    !form.name.trim() ||
    !form.repositoryId ||
    (form.kind === LoopKind.adapter_event &&
      (!(form.connectionId && form.sourceMatcher.trim()) || connectionItems.length === 0)) ||
    (form.kind === LoopKind.schedule &&
      !(form.cronExpression.trim() && form.messageTemplate.trim()))

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label>Kind</Label>
          <Select
            value={form.kind}
            onValueChange={(kind) => setForm((prev) => ({ ...prev, kind: kind as LoopKind }))}
            disabled={isEditing}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={LoopKind.adapter_event}>Subscription</SelectItem>
              <SelectItem value={LoopKind.schedule}>Schedule</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label>Name</Label>
          <Input
            value={form.name}
            onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
            placeholder="What to call this loop"
          />
        </div>
      </div>

      {form.kind === LoopKind.adapter_event ? (
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
            onValueChange={(repositoryId) =>
              setForm((prev) => ({ ...prev, repositoryId, workspaceId: '' }))
            }
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
          <Label>Workspace</Label>
          <Select
            value={form.workspaceId || ANY_WORKSPACE}
            onValueChange={(workspaceId) =>
              setForm((prev) => ({
                ...prev,
                workspaceId: workspaceId === ANY_WORKSPACE ? '' : workspaceId,
              }))
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ANY_WORKSPACE}>Any placing this repository</SelectItem>
              {workspaceItems.map((workspace) => (
                <SelectItem key={workspace.id} value={workspace.id}>
                  {workspace.name}
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
        <div className="space-y-2">
          <Label>Skills</Label>
          <Input
            value={form.skillsText}
            onChange={(event) => setForm((prev) => ({ ...prev, skillsText: event.target.value }))}
            placeholder="e2e-testing, moat-cli"
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

      {form.kind === LoopKind.adapter_event && (
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
        {form.kind === LoopKind.adapter_event && (
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

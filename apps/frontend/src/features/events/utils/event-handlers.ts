import type { QueryClient } from '@tanstack/react-query'
import { refreshNewestMessagesPage } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { applySubagentProgress, upsertMessageByUuid } from '@/lib/api/task-cache'
import { extractToolNames, hasTools } from '@/lib/types/content-guards'
import type { Event, MessageEvent } from '@/lib/types/events'
import type { MoatlessToolName } from '@/lib/types/tools'

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

type InvalidationMapping = Readonly<Record<string, readonly InvalidationKeyFunction[]>>

// SubType-aware invalidation mapping - prevents unnecessary refetches.
//
// IMPORTANT: messages are NOT invalidated here. Invalidating an infinite
// query refetches every loaded page and resets the user's scroll position.
// Events that affect messages take a targeted refetch of the newest page via
// `invalidateMessagesForEvent` below.

export const EVENT_INVALIDATION_MAP = {
  task: {
    // Task-level status changes always affect task detail
    '*': [queryKeys.tasks.detail],
  },
  taskTurn: {
    // Turn status changes affect task detail and potentially git
    created: [queryKeys.tasks.detail],
    active: [queryKeys.tasks.detail],
    completed: [queryKeys.tasks.detail],
    error: [queryKeys.tasks.detail],
    updated: [queryKeys.tasks.detail], // Screenshot updates need task detail refresh
  },
  agent: {
    // Agent subtypes have different invalidation needs
    started: [queryKeys.tasks.liveStatus, queryKeys.tasks.sandboxStatus],
    stopped: [queryKeys.tasks.liveStatus, queryKeys.tasks.sandboxStatus],
    session_init: [], // Newest-page refetch instead (see MESSAGE_REFRESH_TRIGGERS)
    message: [], // Newest-page refetch instead (see MESSAGE_REFRESH_TRIGGERS)
    completed: [queryKeys.tasks.detail, queryKeys.tasks.liveStatus, queryKeys.tasks.sandboxStatus], // Full turn impact
    error: [queryKeys.tasks.detail, queryKeys.tasks.liveStatus, queryKeys.tasks.sandboxStatus],
  },
  sandbox: {
    // Sandbox lifecycle changes affect sandbox status and setup
    '*': [queryKeys.tasks.liveStatus, queryKeys.tasks.sandboxStatus, queryKeys.tasks.sandboxSetup],
  },
  server: {
    // Server events affect sandbox status (servers are embedded in sandbox status)
    '*': [queryKeys.tasks.sandboxStatus],
  },
  navigation: {},
  preview: {
    // Preview events are emitted by backend but not handled by frontend
  },
  resources: {
    // Resource metrics are handled via direct cache updates, no invalidation needed
  },
  message: {
    // Derived message.upsert / subagentProgress apply via applyMessageEvent — no invalidation
  },
} as const satisfies Record<Event['type'], InvalidationMapping>

// Status-affecting events consider subType for granular control
export const STATUS_AFFECTING_SUBTYPES = {
  task: ['created', 'open', 'closed', 'updated', 'metadata.updated'], // All task subtypes affect status
  taskTurn: ['active', 'completed', 'error', 'updated'], // Execution changes and screenshot updates affect status
  agent: ['started', 'stopped', 'completed', 'error'], // Only state changes, not messages
  sandbox: ['ready', 'error', 'removed'], // Only final states affect status
  server: ['starting', 'started', 'stopped', 'failed'], // Only state changes affect status
  navigation: [],
  preview: [], // Preview events don't affect task status
  resources: [], // Resource metrics don't affect task status
  message: [], // Derived UI-only events don't affect task status
} as const satisfies Record<Event['type'], readonly string[]>

export function getInvalidationKeys(event: Event): InvalidationKeyFunction[] {
  const typeMapping = EVENT_INVALIDATION_MAP[event.type]
  if (!typeMapping) {
    return []
  }

  const subTypeKeys =
    event.subType && event.subType in typeMapping
      ? (typeMapping as Record<string, readonly InvalidationKeyFunction[]>)[event.subType]
      : undefined
  const wildcardKeys =
    '*' in typeMapping
      ? (typeMapping as Record<string, readonly InvalidationKeyFunction[]>)['*']
      : undefined

  const keys = subTypeKeys ?? wildcardKeys
  return keys ? [...keys] : []
}

export function shouldInvalidateTaskList(event: Event): boolean {
  // Task lists should be invalidated for status-affecting events
  const subtypes = STATUS_AFFECTING_SUBTYPES[event.type]
  if (!subtypes) {
    return false
  }

  // If no subType specified, consider it status-affecting
  if (!event.subType) {
    return true
  }

  // Check if the specific subType affects status
  return (subtypes as readonly string[]).includes(event.subType)
}

export function isStatusAffectingEvent(event: Event): boolean {
  return shouldInvalidateTaskList(event)
}

/**
 * Tool invalidation registry - maps tool names to query invalidation actions
 * Add new tools here to automatically trigger cache invalidations
 */
type ToolInvalidationAction = (taskId: string, queryClient: QueryClient) => void

const invalidateFiles: ToolInvalidationAction = (taskId, queryClient) => {
  queryClient.invalidateQueries({
    queryKey: queryKeys.tasks.files(taskId),
    refetchType: 'active',
  })
}

const TOOL_INVALIDATION_REGISTRY: Readonly<
  Partial<Record<string | MoatlessToolName, ToolInvalidationAction>>
> = {
  // File editing tools - invalidate file queries
  Edit: invalidateFiles,
  MultiEdit: invalidateFiles,
  Write: invalidateFiles,
  // A shell command can add, move or delete files just as readily (installs,
  // git checkouts, generators), and this event is what refreshes the file tree
  // for them.
  Bash: invalidateFiles,
  // Task metadata updates - invalidate task detail (using clean tool name)
  updateTaskMetadata: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.detail(taskId),
      refetchType: 'active',
    })
  },
  // User input tools - newest-page refetch via MESSAGE_REFRESH_TOOL_TRIGGERS;
  // no-op here so the invalidation registry doesn't reset the infinite query.
  AskUserQuestion: () => {
    /* handled by invalidateMessagesForEvent */
  },
  ExitPlanMode: () => {
    /* handled by invalidateMessagesForEvent */
  },
} as const

/**
 * Turn-final events that reconcile the newest messages page — the firehose-drop
 * safety net beside live `message.upsert` (server-owned assembly). Streaming `agent.message`
 * is deliberately absent: the server-assembled `message.upsert` keeps the Chat
 * live, so there is no per-message refetch.
 */
const MESSAGE_REFRESH_EVENT_TRIGGERS: ReadonlyArray<{ type: Event['type']; subType: string }> = [
  { type: 'taskTurn', subType: 'completed' },
  { type: 'taskTurn', subType: 'error' },
  { type: 'agent', subType: 'session_init' },
]

/**
 * Apply a derived `message.*` event to the messages cache (server-owned assembly and skeleton-message model — the
 * frontend renders, it does not assemble). `upsert` replaces-or-inserts the
 * server-assembled `UiMessage` by uuid; `subagentProgress` writes an absolute
 * subagent summary through onto the cached parent's `Agent` block. No-op when
 * the task has no cache entry — the read path hydrates it on open, so we never
 * fabricate a partial history from one event.
 */
export function applyMessageEvent(event: MessageEvent, queryClient: QueryClient): void {
  if (!event.taskId) {
    return
  }
  if (event.subType === 'subagentProgress') {
    // Write the absolute summary through onto the cached parent's `Agent`
    // block (idempotent), then debounce-refetch the transcript so an expanded
    // card refreshes live without streaming subagent content.
    applySubagentProgress(queryClient, event.taskId, event.payload)
    scheduleSubagentTranscriptRefetch(queryClient, event.taskId, event.payload.toolUseId)
    return
  }
  upsertMessageByUuid(queryClient, event.taskId, event.payload)
}

/**
 * Debounce transcript refetches per `(taskId, toolUseId)`: a burst of
 * `subagentProgress` ticks coalesces into one refetch of the (expanded)
 * transcript query. A collapsed card's query is inactive, so it is only marked
 * stale and reloads the next time the card is expanded.
 */
const SUBAGENT_REFETCH_DEBOUNCE_MS = 300
const subagentRefetchTimers = new Map<string, ReturnType<typeof setTimeout>>()

function scheduleSubagentTranscriptRefetch(
  queryClient: QueryClient,
  taskId: string,
  toolUseId: string,
): void {
  const key = `${taskId}:${toolUseId}`
  const existing = subagentRefetchTimers.get(key)
  if (existing) {
    clearTimeout(existing)
  }
  const timer = setTimeout(() => {
    subagentRefetchTimers.delete(key)
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.subagentTranscript(taskId, toolUseId),
    })
  }, SUBAGENT_REFETCH_DEBOUNCE_MS)
  subagentRefetchTimers.set(key, timer)
}

/**
 * Reconcile the newest messages page on turn-final events — the firehose-drop
 * safety net beside live `message.upsert`. Only fetches when the chat view is
 * mounted (an observer exists); otherwise marks the query stale so it reloads
 * lazily the next time the user opens that task's chat.
 */
export function invalidateMessagesForEvent(event: Event, queryClient: QueryClient): void {
  if (!event.taskId) {
    return
  }

  const matchesTrigger = MESSAGE_REFRESH_EVENT_TRIGGERS.some(
    (trigger) => trigger.type === event.type && trigger.subType === event.subType,
  )
  if (!matchesTrigger) {
    return
  }

  const messagesKey = queryKeys.tasks.messages(event.taskId)
  const observerCount =
    queryClient.getQueryCache().find({ queryKey: messagesKey })?.getObserversCount() ?? 0

  if (observerCount === 0) {
    queryClient.invalidateQueries({ queryKey: messagesKey, refetchType: 'none' })
    return
  }

  refreshNewestMessagesPage(queryClient, event.taskId).catch(() => {
    /* swallow: refetch failures surface via the next stream event */
  })
}

/**
 * Extract the tool names an `agent.message` source event carries.
 *
 * Prefers the skeleton's `toolNames` field (the agent.message skeleton — the source event's
 * persisted/relayed form is a metadata skeleton, and tool-invalidation reads a
 * field, not content). Falls back to parsing `message.content` for the
 * transitional window before the relay is slimmed / rows are backfilled; that
 * fallback drops out once one shape remains.
 */
function toolNamesFromAgentMessage(payload: unknown): string[] {
  const shaped = payload as { toolNames?: unknown; message?: { content?: unknown } }
  if (Array.isArray(shaped?.toolNames)) {
    return shaped.toolNames.filter((name): name is string => typeof name === 'string')
  }
  const content = shaped?.message?.content
  if (content && Array.isArray(content) && hasTools(content)) {
    return extractToolNames(content)
  }
  return []
}

/**
 * Invalidate queries based on tools used in an agent message event
 * Extracts tools from event and dispatches to appropriate invalidation actions
 */
export function invalidateByTool(event: Event, queryClient: QueryClient): void {
  // Only process agent message events
  if (event.type !== 'agent' || event.subType !== 'message') {
    return
  }

  for (const toolName of toolNamesFromAgentMessage(event.payload)) {
    const action = TOOL_INVALIDATION_REGISTRY[toolName]
    if (action) {
      action(event.taskId, queryClient)
    }
  }
}

import type { QueryClient } from '@tanstack/react-query'
import { refreshNewestMessagesPage } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { removeMessage, upsertMessageByUuid } from '@/lib/api/task-cache'
import { extractToolNames, hasTools } from '@/lib/types/content-guards'
import type { Event, MessageEvent } from '@/lib/types/events'
import type { MoatlessToolName } from '@/lib/types/tools'

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

type InvalidationMapping = Readonly<Record<string, readonly InvalidationKeyFunction[]>>

// SubType-aware invalidation mapping - prevents unnecessary refetches.
//
// IMPORTANT: messages are NOT invalidated here. Invalidating an infinite
// query refetches every loaded page and resets the user's scroll position.
// The set of events that previously triggered messages-key invalidation now
// triggers a targeted refetch of the newest page via
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
    started: [queryKeys.tasks.sandboxStatus],
    stopped: [queryKeys.tasks.sandboxStatus],
    session_init: [], // Newest-page refetch instead (see MESSAGE_REFRESH_TRIGGERS)
    message: [], // Newest-page refetch instead (see MESSAGE_REFRESH_TRIGGERS)
    completed: [queryKeys.tasks.detail, queryKeys.tasks.sandboxStatus], // Full turn impact
    error: [queryKeys.tasks.detail, queryKeys.tasks.sandboxStatus],
  },
  sandbox: {
    // Sandbox lifecycle changes affect sandbox status and setup
    '*': [queryKeys.tasks.sandboxStatus, queryKeys.tasks.sandboxSetup],
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
    // Derived message.upsert/deleted apply by uuid via applyMessageEvent — no invalidation
  },
} as const satisfies Record<Event['type'], InvalidationMapping>

// Status-affecting events now consider subType for granular control
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

const TOOL_INVALIDATION_REGISTRY: Readonly<
  Partial<Record<string | MoatlessToolName, ToolInvalidationAction>>
> = {
  // File editing tools - invalidate file queries
  Edit: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.files(taskId),
      refetchType: 'active',
    })
  },
  MultiEdit: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.files(taskId),
      refetchType: 'active',
    })
  },
  Write: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.files(taskId),
      refetchType: 'active',
    })
  },
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
 * safety net beside live `message.upsert` (ADR-0002). Streaming `agent.message`
 * is deliberately absent: the server-assembled `message.upsert` keeps the Chat
 * live, so there is no per-message refetch.
 */
const MESSAGE_REFRESH_EVENT_TRIGGERS: ReadonlyArray<{ type: Event['type']; subType: string }> = [
  { type: 'taskTurn', subType: 'completed' },
  { type: 'taskTurn', subType: 'error' },
  { type: 'agent', subType: 'session_init' },
]

/**
 * Apply a derived `message.upsert` / `message.deleted` event to the messages
 * cache by uuid (ADR-0002 — the frontend renders, it does not assemble). The
 * server assembles the `UiMessage`; we replace-or-insert (or remove) it in the
 * task's existing cache. No-op when the task has no cache entry — the read path
 * hydrates it on open, so we never fabricate a partial history from one event.
 */
export function applyMessageEvent(event: MessageEvent, queryClient: QueryClient): void {
  if (!event.taskId) {
    return
  }
  if (event.subType === 'deleted') {
    removeMessage(queryClient, event.taskId, event.payload.uuid)
    return
  }
  upsertMessageByUuid(queryClient, event.taskId, event.payload)
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
 * Invalidate queries based on tools used in an agent message event
 * Extracts tools from event and dispatches to appropriate invalidation actions
 */
export function invalidateByTool(event: Event, queryClient: QueryClient): void {
  // Only process agent message events
  if (event.type !== 'agent' || event.subType !== 'message') {
    return
  }

  // Extract message content from event payload
  const payload = event.payload as { message?: { content?: unknown } }
  const message = payload?.message

  if (!(message?.content && Array.isArray(message.content))) {
    return
  }

  // Check if event has any tools
  if (!hasTools(message.content)) {
    return
  }

  // Extract tool names and trigger invalidations
  const toolNames = extractToolNames(message.content)

  for (const toolName of toolNames) {
    const action = TOOL_INVALIDATION_REGISTRY[toolName]
    if (action) {
      action(event.taskId, queryClient)
    }
  }
}

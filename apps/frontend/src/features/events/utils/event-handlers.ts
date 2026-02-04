import type { Event, MoatlessToolName } from '@moatless/types'
import { extractToolNames, hasTools } from '@moatless/types'
import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

type InvalidationMapping = Readonly<Record<string, readonly InvalidationKeyFunction[]>>

// SubType-aware invalidation mapping - prevents unnecessary refetches

export const EVENT_INVALIDATION_MAP = {
  task: {
    // Task-level status changes always affect task detail
    '*': [queryKeys.tasks.detail],
  },
  taskTurn: {
    // Turn status changes affect task detail and potentially git
    created: [queryKeys.tasks.detail],
    active: [queryKeys.tasks.detail],
    completed: [queryKeys.tasks.detail, queryKeys.tasks.messages],
    error: [queryKeys.tasks.detail, queryKeys.tasks.messages],
    updated: [queryKeys.tasks.detail], // Screenshot updates need task detail refresh
  },
  agent: {
    // Agent subtypes have different invalidation needs
    started: [queryKeys.tasks.workspaceStatus],
    stopped: [queryKeys.tasks.workspaceStatus],
    message: [queryKeys.tasks.messages], // Only messages, not status
    completed: [queryKeys.tasks.detail], // Full turn impact
    error: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus],
  },
  workspace: {
    // Workspace operations affect multiple areas
    '*': [queryKeys.tasks.workspaceStatus, queryKeys.tasks.workspaceSetup],
  },
  environment: {
    // Environment changes only affect workspace status
    '*': [queryKeys.tasks.workspaceStatus],
  },
  server: {
    // Server events affect workspace status (servers are embedded in workspace status)
    '*': [queryKeys.tasks.workspaceStatus],
  },
  navigation: {},
  preview: {
    // Preview events are emitted by backend but not handled by frontend
  },
} as const satisfies Record<Event['type'], InvalidationMapping>

// Status-affecting events now consider subType for granular control
export const STATUS_AFFECTING_SUBTYPES = {
  task: ['created', 'open', 'closed', 'updated'], // All task subtypes affect status
  taskTurn: ['active', 'completed', 'error', 'updated'], // Execution changes and screenshot updates affect status
  agent: ['started', 'stopped', 'completed', 'error'], // Only state changes, not messages
  environment: ['ready', 'error'], // Only final states affect status
  workspace: ['ready', 'destroyed', 'error'], // Only final states affect status
  server: ['started', 'stopped', 'error'], // Only state changes affect status
  navigation: [],
  preview: [], // Preview events don't affect task status
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
  // User input tools - invalidate messages to refresh with hasPendingUserInput flag
  AskUserQuestion: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.messages(taskId),
      refetchType: 'active',
    })
  },
  ExitPlanMode: (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.messages(taskId),
      refetchType: 'active',
    })
  },
} as const

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

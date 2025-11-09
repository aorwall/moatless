import type { Event } from '@moatless/types'
import { extractToolNames, hasFileEditingTool, hasTools } from '@moatless/types'
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
  taskIteration: {
    // Iteration status changes affect task detail and potentially git
    created: [queryKeys.tasks.detail],
    active: [queryKeys.tasks.detail],
    completed: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff],
    error: [queryKeys.tasks.detail],
    updated: [queryKeys.tasks.detail], // Screenshot updates need task detail refresh
  },
  agent: {
    // Agent subtypes have different invalidation needs
    started: [queryKeys.tasks.workspaceStatus],
    stopped: [queryKeys.tasks.workspaceStatus],
    message: [queryKeys.tasks.messages], // Only messages, not status
    completed: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff], // Full iteration impact
    error: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus],
  },
  workspace: {
    // Workspace operations affect multiple areas
    '*': [queryKeys.tasks.workspaceStatus, queryKeys.tasks.workspaceSetup, queryKeys.tasks.gitDiff],
  },
  environment: {
    // Environment changes only affect workspace status
    '*': [queryKeys.tasks.workspaceStatus],
  },
  server: {
    // Server events only affect server status
    '*': [queryKeys.tasks.serverStatus],
  },
  navigation: {},
  preview: {
    // Preview events are emitted by backend but not handled by frontend
  },
} as const satisfies Record<Event['type'], InvalidationMapping>

// Status-affecting events now consider subType for granular control
export const STATUS_AFFECTING_SUBTYPES = {
  task: ['created', 'open', 'closed'], // All task subtypes affect status
  taskIteration: ['active', 'completed', 'error', 'updated'], // Execution changes and screenshot updates affect status
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
 * Check if an event contains file-editing tool calls that require file query invalidation
 * Detects Edit, MultiEdit, and Write tools in agent message events
 * @deprecated Use invalidateByTool instead
 */
export function shouldInvalidateFiles(event: Event): boolean {
  // Only check agent message events
  if (event.type !== 'agent' || event.subType !== 'message') {
    return false
  }

  // Access the message content from the event payload
  // For agent message events, payload contains the ClaudeCodeSDKMessage with a message field
  const payload = event.payload as { message?: { content?: unknown } }
  const message = payload?.message

  if (!(message?.content && Array.isArray(message.content))) {
    return false
  }

  // Use the helper from types package to detect file-editing tools
  return hasFileEditingTool(message.content)
}

/**
 * Tool invalidation registry - maps tool names to query invalidation actions
 * Add new tools here to automatically trigger cache invalidations
 */
type ToolInvalidationAction = (taskId: string, queryClient: QueryClient) => void

const TOOL_INVALIDATION_REGISTRY: Readonly<Record<string, ToolInvalidationAction>> = {
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
  // Task metadata updates - invalidate task detail
  'mcp__moatless-tools__updateTaskMetadata': (taskId, queryClient) => {
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.detail(taskId),
      refetchType: 'active',
    })
  },
  // Add more tool invalidations here:
  // WebFetch: (taskId, queryClient) => { ... },
  // Bash: (taskId, queryClient) => { ... },
  // etc.
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

  if (toolNames.length > 0) {
    // biome-ignore lint/suspicious/noConsole: Intentional logging for debugging tool invalidations
    console.log('[invalidateByTool] Extracted tool names:', toolNames)
  }

  for (const toolName of toolNames) {
    const action = TOOL_INVALIDATION_REGISTRY[toolName]
    if (action) {
      // biome-ignore lint/suspicious/noConsole: Intentional logging for debugging tool invalidations
      console.log(`[invalidateByTool] Triggering invalidation for tool: ${toolName}`)
      action(event.taskId, queryClient)
    }
  }
}

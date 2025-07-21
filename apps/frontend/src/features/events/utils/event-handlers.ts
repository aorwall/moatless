import type { Event } from '@moatless/types'
import { queryKeys } from '@/lib/api/query-keys'

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
} as const

// Status-affecting events now consider subType for granular control
export const STATUS_AFFECTING_SUBTYPES = {
  task: ['created', 'open', 'closed'], // All task subtypes affect status
  taskIteration: ['active', 'completed', 'error'], // Only execution changes affect status
  agent: ['started', 'stopped', 'error'], // Only state changes, not messages
  environment: ['ready', 'error'], // Only final states affect status
  workspace: ['ready', 'destroyed', 'error'], // Only final states affect status
  server: ['started', 'stopped', 'error'], // Only state changes affect status
} as const

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

export function getInvalidationKeys(event: Event): InvalidationKeyFunction[] {
  const typeMapping = EVENT_INVALIDATION_MAP[event.type as keyof typeof EVENT_INVALIDATION_MAP]
  if (!typeMapping) return []

  // Check for subType-specific mapping
  if (typeof typeMapping === 'object' && !Array.isArray(typeMapping)) {
    const subTypeKeys = event.subType && typeMapping[event.subType as keyof typeof typeMapping]
    const wildcardKeys = typeMapping['*']

    // Use subType-specific keys if available, otherwise fall back to wildcard
    const keys = subTypeKeys || wildcardKeys
    return keys ? [...keys] : []
  }

  // Legacy support for flat mapping (shouldn't happen with new schema)
  return Array.isArray(typeMapping) ? [...typeMapping] : []
}

export function shouldInvalidateTaskList(event: Event): boolean {
  // Task lists should be invalidated for status-affecting events
  const subtypes = STATUS_AFFECTING_SUBTYPES[event.type as keyof typeof STATUS_AFFECTING_SUBTYPES]
  if (!subtypes) return false

  // If no subType specified, consider it status-affecting
  if (!event.subType) return true

  // Check if the specific subType affects status
  return subtypes.includes(event.subType as any)
}

export function isStatusAffectingEvent(event: Event): boolean {
  return shouldInvalidateTaskList(event)
}

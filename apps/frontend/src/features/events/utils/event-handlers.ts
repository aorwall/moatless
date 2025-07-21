import type { Event } from '@moatless/types'
import { queryKeys } from '@/lib/api/query-keys'

export const STATUS_AFFECTING_EVENTS = ['error', 'task_status', 'environment', 'agent'] as const

// Map of actively used event types to their invalidation keys
// These invalidation keys are for the specific task only
export const EVENT_INVALIDATION_MAP = {
  message: [queryKeys.tasks.messages], // Invalidate messages when new messages arrive
  result: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff], // Keep for iteration updates and git changes
  error: [queryKeys.tasks.detail],
  workspace: [
    queryKeys.tasks.workspaceStatus,
    queryKeys.tasks.workspaceSetup,
    queryKeys.tasks.gitDiff,
  ],
  environment: [queryKeys.tasks.workspaceStatus], // Environment status changes only affect workspace
  task_status: [queryKeys.tasks.detail], // Main task status invalidation
  agent: [queryKeys.tasks.workspaceStatus], // Agent status changes only affect workspace
  // Deprecated/unused event types:
  // INPUT - removed, replaced with TASK_STATUS for status changes
  // START, STOP, SERVER, TOOL, COMPLETION - not actively used in production
  start: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus],
  stop: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus, queryKeys.tasks.serverStatus],
  server: [queryKeys.tasks.serverStatus],
  tool: [queryKeys.tasks.events, queryKeys.tasks.messages], // Also invalidate messages for tool events
  completion: [queryKeys.tasks.detail],
} as const

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

export function getInvalidationKeys(event: Event): InvalidationKeyFunction[] {
  const keys = EVENT_INVALIDATION_MAP[event.type]
  return keys ? [...keys] : []
}

export function shouldInvalidateTaskList(event: Event): boolean {
  // Task lists should be invalidated for status-affecting events
  return STATUS_AFFECTING_EVENTS.some((type) => type === event.type)
}

export function isStatusAffectingEvent(event: Event): boolean {
  return STATUS_AFFECTING_EVENTS.some((type) => type === event.type)
}

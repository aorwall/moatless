import type { Event } from '@moatless/types'
import { EventType } from '@moatless/types'
import { queryKeys } from '@/lib/api/query-keys'

export const STATUS_AFFECTING_EVENTS = [EventType.ERROR, EventType.TASK_STATUS] as const

// Map of actively used event types to their invalidation keys
// These invalidation keys are for the specific task only
export const EVENT_INVALIDATION_MAP = {
  [EventType.MESSAGE]: [queryKeys.tasks.messages], // Invalidate messages when new messages arrive
  [EventType.RESULT]: [queryKeys.tasks.detail, queryKeys.tasks.gitDiff], // Keep for iteration updates and git changes
  [EventType.ERROR]: [queryKeys.tasks.detail],
  [EventType.WORKSPACE]: [
    queryKeys.tasks.workspaceStatus,
    queryKeys.tasks.workspaceSetup,
    queryKeys.tasks.gitDiff,
  ],
  [EventType.TASK_STATUS]: [queryKeys.tasks.detail], // Main task status invalidation
  // Deprecated/unused event types:
  // INPUT - removed, replaced with TASK_STATUS for status changes
  // START, STOP, SERVER, TOOL, COMPLETION - not actively used in production
  [EventType.START]: [queryKeys.tasks.detail, queryKeys.tasks.workspaceStatus],
  [EventType.STOP]: [
    queryKeys.tasks.detail,
    queryKeys.tasks.workspaceStatus,
    queryKeys.tasks.serverStatus,
  ],
  [EventType.SERVER]: [queryKeys.tasks.serverStatus],
  [EventType.TOOL]: [queryKeys.tasks.events, queryKeys.tasks.messages], // Also invalidate messages for tool events
  [EventType.COMPLETION]: [queryKeys.tasks.detail],
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

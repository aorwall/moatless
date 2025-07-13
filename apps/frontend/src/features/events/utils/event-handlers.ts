import type { Event } from '@moatless/shared/types/event'
import { EventType } from '@moatless/shared/types/event'
import { queryKeys } from '@/lib/api/query-keys'

export const STATUS_AFFECTING_EVENTS = [
  EventType.RESULT,
  EventType.ERROR,
  EventType.INPUT,
  EventType.TASK_STATUS,
] as const

// Wrapper functions for constants that need to be used as invalidation keys
const tasksAllKey = (_id: string) => queryKeys.tasks.all

// Map of actively used event types to their invalidation keys
// Unused event types (START, STOP, SERVER, TOOL, COMPLETION) have been removed
export const EVENT_INVALIDATION_MAP = {
  [EventType.MESSAGE]: [], // No invalidation needed - handled by direct cache update
  [EventType.RESULT]: [queryKeys.task, queryKeys.taskGitDiff, tasksAllKey],
  [EventType.ERROR]: [queryKeys.task],
  [EventType.WORKSPACE]: [queryKeys.taskWorkspaceStatus, queryKeys.taskGitDiff],
  [EventType.INPUT]: [queryKeys.task, tasksAllKey],
  [EventType.TASK_STATUS]: [queryKeys.task, tasksAllKey],
  // Deprecated/unused event types:
  // START, STOP, SERVER, TOOL, COMPLETION - not actively used in production
  [EventType.START]: [queryKeys.task, queryKeys.taskWorkspaceStatus],
  [EventType.STOP]: [queryKeys.task, queryKeys.taskWorkspaceStatus, queryKeys.taskServerStatus],
  [EventType.SERVER]: [queryKeys.taskServerStatus],
  [EventType.TOOL]: [queryKeys.taskEvents],
  [EventType.COMPLETION]: [queryKeys.task],
} as const

export type InvalidationKeyFunction = (id: string) => readonly unknown[]

export function getInvalidationKeys(event: Event): InvalidationKeyFunction[] {
  const keys = EVENT_INVALIDATION_MAP[event.type]
  return keys ? [...keys] : []
}

export function shouldInvalidateTaskList(event: Event): boolean {
  // Task lists should be invalidated for status-affecting events
  return STATUS_AFFECTING_EVENTS.includes(event.type as any)
}

export function isStatusAffectingEvent(event: Event): boolean {
  return STATUS_AFFECTING_EVENTS.includes(event.type as any)
}

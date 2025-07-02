import { queryKeys } from '@/lib/api/query-keys'
import { EventType } from '@moatless/shared/types/event'
import type { Event } from '@moatless/shared/types/event'

export const STATUS_AFFECTING_EVENTS = [
  EventType.RESULT,
  EventType.ERROR,
  EventType.STOP,
  EventType.START,
] as const

export const EVENT_INVALIDATION_MAP = {
  [EventType.START]: [
    queryKeys.task,
    queryKeys.taskWorkspaceStatus,
  ],
  [EventType.STOP]: [
    queryKeys.task,
    queryKeys.taskWorkspaceStatus,
    queryKeys.taskServerStatus,
  ],
  [EventType.RESULT]: [
    queryKeys.task,
    queryKeys.taskGitDiff,
  ],
  [EventType.ERROR]: [
    queryKeys.task,
  ],
  [EventType.WORKSPACE]: [
    queryKeys.taskWorkspaceStatus,
    queryKeys.taskGitDiff,
  ],
  [EventType.SERVER]: [
    queryKeys.taskServerStatus,
  ],
  [EventType.MESSAGE]: [
    queryKeys.taskMessages,
  ],
  [EventType.INPUT]: [],
  [EventType.TOOL]: [
    queryKeys.taskEvents,
  ],
  [EventType.COMPLETION]: [
    queryKeys.task,
  ],
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
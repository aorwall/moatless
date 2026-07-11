import type { Event } from '@/lib/types/events'
import type { LifecycleEvent } from '../types'

/** Coerce a payload value to a string only when present (null/undefined → undefined). */
function optString(value: unknown): string | undefined {
  return value != null ? String(value) : undefined
}

/**
 * Map a raw stream `Event` to a UI `LifecycleEvent` for the Events tab. Returns
 * null for events that don't belong in the lifecycle log.
 */
export function toLifecycleEvent(event: Event): LifecycleEvent | null {
  const rawSubType = (event as { subType?: string }).subType
  // taskTurn events are turn-level facts; only `turn_interrupted` belongs in the
  // sandbox/server lifecycle log — the rest (active/completed/…) are filtered out.
  const isTurnInterrupted = event.type === 'taskTurn' && rawSubType === 'turn_interrupted'
  if (event.type !== 'sandbox' && event.type !== 'server' && !isTurnInterrupted) {
    return null
  }
  const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
  const exitCodeRaw = payload['exitCode']
  return {
    eventId: event.eventId,
    type: event.type as 'sandbox' | 'server' | 'taskTurn',
    subType: rawSubType ?? '',
    timestamp: event.timestamp ? new Date(event.timestamp).toISOString() : new Date().toISOString(),
    isError: event.isError,
    message: optString(payload['message']),
    reason: optString(payload['reason']),
    serverName:
      event.type === 'server' && payload['serverName'] != null
        ? String(payload['serverName'])
        : undefined,
    nodeName: optString(payload['nodeName']),
    disruptionReason: optString(payload['disruptionReason']),
    initiator: optString(payload['initiator']),
    exitCode: typeof exitCodeRaw === 'number' ? exitCodeRaw : undefined,
    terminatedReason: optString(payload['terminatedReason']),
    pendingSeconds:
      typeof payload['pendingSeconds'] === 'number' ? payload['pendingSeconds'] : undefined,
    payload,
  }
}

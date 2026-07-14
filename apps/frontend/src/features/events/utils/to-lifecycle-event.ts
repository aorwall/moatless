import type { Event } from '@/lib/types/events'
import type { LifecycleEvent } from '../types'

/** Coerce a payload value to a string only when present (null/undefined → undefined). */
function optString(value: unknown): string | undefined {
  return value != null ? String(value) : undefined
}

/**
 * Extract a display message from `payload.message`. Most events carry a plain
 * string, but `agent.message` carries the SDK message *object* (`{ role, … }`)
 * — stringifying that would render `[object Object]`, so we surface just the
 * role (e.g. "assistant message") and leave the content to the chat view.
 */
function extractMessage(raw: unknown): string | undefined {
  if (typeof raw === 'string') {
    return raw
  }
  if (raw && typeof raw === 'object') {
    const role = (raw as { role?: unknown }).role
    return typeof role === 'string' ? `${role} message` : undefined
  }
  return
}

/** Event types shown in the log viewer — every persisted type, but not the
 *  live-only UI streams (resources/navigation/preview). */
const LOG_TYPES = new Set(['sandbox', 'server', 'taskTurn', 'agent', 'task'])

/**
 * Map a raw stream `Event` to a UI `LifecycleEvent` for the Events tab. Returns
 * null for the live-only UI streams (resources/navigation/preview) that aren't
 * part of the persisted event log.
 */
export function toLifecycleEvent(event: Event): LifecycleEvent | null {
  const rawSubType = (event as { subType?: string }).subType
  if (!LOG_TYPES.has(event.type)) {
    return null
  }
  const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
  const exitCodeRaw = payload['exitCode']
  return {
    eventId: event.eventId,
    type: event.type as LifecycleEvent['type'],
    subType: rawSubType ?? '',
    timestamp: event.timestamp ? new Date(event.timestamp).toISOString() : new Date().toISOString(),
    isError: event.isError,
    message: extractMessage(payload['message']),
    agentType:
      event.type === 'agent' ? optString((event as { agentType?: unknown }).agentType) : undefined,
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

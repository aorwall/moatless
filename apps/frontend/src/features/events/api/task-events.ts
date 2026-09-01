import { apiClient } from '@/lib/api/client'
import type { Event } from '@/lib/types/events'
import type { LifecycleEvent } from '../types'
import { toLifecycleEvent } from '../utils/to-lifecycle-event'

/**
 * How many of a task's most recent events the log viewer loads. The endpoint
 * returns the newest `limit` rows (oldest-first), so this is a tail window, not
 * a truncation of the start.
 *
 * Kept small on purpose: each event carries an arbitrary JSON payload, so a
 * larger window across several visited tasks costs multi-second
 * cycle-collection pauses.
 */
const HISTORY_LIMIT = 300

/**
 * Fetch a task's recent event history, already mapped to the compact UI shape.
 *
 * Mapping happens here rather than in the consumer so the raw `Event[]` — whose
 * payloads include whole agent SDK messages — is garbage immediately instead of
 * being retained by the query cache alongside the mapped copy.
 */
export async function fetchTaskEvents(taskId: string): Promise<LifecycleEvent[]> {
  const response = await apiClient.get<{ items: Event[] }>(
    `/api/v1/tasks/${taskId}/events/history`,
    // No `types` filter — the log viewer shows every persisted event type
    // (agent, task, taskTurn, sandbox, server). `toLifecycleEvent` still drops
    // the live-only UI streams (resources/navigation/preview) that never reach
    // this endpoint anyway.
    { limit: HISTORY_LIMIT },
  )
  return response.items
    .map(toLifecycleEvent)
    .filter((event): event is LifecycleEvent => event !== null)
}

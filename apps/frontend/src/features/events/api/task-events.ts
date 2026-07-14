import { apiClient } from '@/lib/api/client'
import type { Event } from '@/lib/types/events'

/** Upper bound requested from the history endpoint (its own hard cap is 5000). */
const HISTORY_LIMIT = 2000

export async function fetchTaskEvents(taskId: string): Promise<Event[]> {
  const response = await apiClient.get<{ items: Event[] }>(
    `/api/v1/tasks/${taskId}/events/history`,
    // No `types` filter — the log viewer shows every persisted event type
    // (agent, task, taskTurn, sandbox, server). `toLifecycleEvent` still drops
    // the live-only UI streams (resources/navigation/preview) that never reach
    // this endpoint anyway. A generous limit keeps busy tasks from truncating.
    { limit: HISTORY_LIMIT },
  )
  return response.items
}

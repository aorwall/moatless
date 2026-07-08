import { apiClient } from '@/lib/api/client'
import type { Event } from '@/lib/types/events'

export async function fetchTaskEvents(taskId: string): Promise<Event[]> {
  const response = await apiClient.get<{ items: Event[] }>(
    `/api/v1/tasks/${taskId}/events/history`,
    // taskTurn is included for `turn_interrupted`; toSandboxEvent filters the
    // other turn subtypes back out so the log only gains the interruption fact.
    { types: 'sandbox,server,taskTurn' },
  )
  return response.items
}

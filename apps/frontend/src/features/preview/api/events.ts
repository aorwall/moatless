import { apiClient } from '@/lib/api/client'
import type { Event } from '@/lib/types/events'

export async function fetchTaskEvents(taskId: string): Promise<Event[]> {
  const response = await apiClient.get<{ items: Event[] }>(
    `/api/v1/tasks/${taskId}/events/history`,
    { types: 'sandbox,server' },
  )
  return response.items
}

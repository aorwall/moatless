import { apiClient } from '@/lib/api/client'
import type { Event } from '@/lib/types/events'

export function fetchTaskEvents(taskId: string): Promise<Event[]> {
  return apiClient.get(`/api/v1/tasks/${taskId}/events`, {
    types: 'sandbox,server',
  })
}

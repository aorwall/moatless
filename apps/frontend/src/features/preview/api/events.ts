import type { Event } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export function fetchTaskEvents(taskId: string): Promise<Event[]> {
  return apiClient.get(`/api/v1/tasks/${taskId}/events`, {
    types: 'sandbox,server',
  })
}

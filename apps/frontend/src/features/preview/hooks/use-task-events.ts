import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchTaskEvents } from '../api/events'

export function useHistoricalSandboxEvents(taskId: string | undefined) {
  return useQuery({
    queryKey: queryKeys.tasks.events(taskId ?? ''),
    queryFn: () =>
      taskId ? fetchTaskEvents(taskId) : Promise.reject(new Error('taskId is required')),
    enabled: !!taskId,
    staleTime: 30_000,
    refetchOnWindowFocus: false,
  })
}

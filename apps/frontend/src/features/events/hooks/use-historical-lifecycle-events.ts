import { useQuery } from '@tanstack/react-query'
import { TASK_DATA_GC_TIME } from '@/lib/api/query-client'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchTaskEvents } from '../api/task-events'

/**
 * Fetch a task's historical sandbox/server lifecycle events so the Events tab
 * and recent-events box persist across page reloads.
 */
export function useHistoricalLifecycleEvents(taskId: string | undefined) {
  return useQuery({
    queryKey: queryKeys.tasks.events(taskId ?? ''),
    queryFn: () =>
      taskId ? fetchTaskEvents(taskId) : Promise.reject(new Error('taskId is required')),
    enabled: !!taskId,
    staleTime: 30_000,
    gcTime: TASK_DATA_GC_TIME,
    refetchOnWindowFocus: false,
  })
}

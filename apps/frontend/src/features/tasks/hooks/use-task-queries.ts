import type { TaskResponse } from '@moatless/api-client'
import { queryOptions, type UseQueryOptions, useQuery } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

export { useTaskLiveStatusQuery } from '@/lib/api/task-live-status-queries'
// Re-exported from the shared api layer so `features/files` can consume the
// sandbox-status query without importing `features/tasks`.
export { useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'

type TaskDetailQueryOptions = Omit<
  UseQueryOptions<TaskResponse, Error, TaskResponse, ReturnType<typeof queryKeys.tasks.detail>>,
  'queryKey' | 'queryFn'
>

/**
 * Colocated `queryKey` + `queryFn` (+ defaults) for a task's detail. Use with
 * `ensureQueryData`/`getQueryData` in route loaders and the SSE provider for
 * typed cache access that stays in sync with the query key.
 */
export function taskDetailQueryOptions(taskId: string) {
  return queryOptions({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    staleTime: 30_000,
    // Rely on SSE events for real-time updates, not aggressive refetching
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
  })
}

export function useTaskDetailQuery(taskId: string, options?: TaskDetailQueryOptions) {
  const { enabled = true, ...restOptions } = options ?? {}

  return useQuery({
    ...taskDetailQueryOptions(taskId),
    enabled: enabled && !!taskId,
    ...restOptions,
  })
}

export function useTaskStatisticsQuery(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.statistics(taskId),
    queryFn: () => tasksApi.getTaskStatistics(taskId),
    staleTime: 30_000,
    enabled: !!taskId,
  })
}

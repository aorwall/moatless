import type { TaskResponse } from '@moatless/api-client'
import { type UseQueryOptions, useQuery } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

// Re-exported from the shared api layer so `features/files` can consume the
// sandbox-status query without importing `features/tasks`.
export { useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'

type TaskDetailQueryOptions = Omit<
  UseQueryOptions<TaskResponse, Error, TaskResponse, ReturnType<typeof queryKeys.tasks.detail>>,
  'queryKey' | 'queryFn'
>

export function useTaskDetailQuery(taskId: string, options?: TaskDetailQueryOptions) {
  const { enabled = true, staleTime = 30_000, ...queryOptions } = options ?? {}

  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: enabled && !!taskId,
    staleTime,
    // Rely on SSE events for real-time updates, not aggressive refetching
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
    ...queryOptions,
  })
}

/**
 * Fetch the task's external connections (Linear issues, GitHub PRs, Slack
 * threads) with decoded labels, live status, and message counts. Requests a
 * status refresh; the backend has its own staleness guard so this stays cheap.
 */
export function useTaskConnectionsQuery(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.connections(taskId),
    queryFn: () => tasksApi.getTaskConnections(taskId, { refresh: true }),
    staleTime: 60_000,
    enabled: !!taskId,
  })
}

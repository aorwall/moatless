import type { TaskResponse, WorkspaceStatusFullResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { workspaceStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import { type UseQueryOptions, useQuery } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

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

type TaskWorkspaceStatusQueryOptions = Omit<
  UseQueryOptions<
    WorkspaceStatusFullResponse,
    Error,
    WorkspaceStatusFullResponse,
    ReturnType<typeof queryKeys.tasks.workspaceStatus>
  >,
  'queryKey' | 'queryFn'
>

export function useTaskWorkspaceStatusQuery(
  taskId: string,
  options?: TaskWorkspaceStatusQueryOptions,
) {
  const {
    enabled = true,
    staleTime = 5000,
    refetchInterval = false,
    ...queryOptions
  } = options ?? {}

  // Polling disabled by default - rely on SSE events for real-time updates
  // Callers can override with refetchInterval if polling is specifically needed

  return useQuery({
    queryKey: queryKeys.tasks.workspaceStatus(taskId),
    queryFn: () => unwrapResponse<WorkspaceStatusFullResponse>(workspaceStatusHandler(taskId)),
    enabled: enabled && !!taskId,
    staleTime,
    refetchInterval,
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
    ...queryOptions,
  })
}

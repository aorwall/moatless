import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { sandboxStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
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

type TaskSandboxStatusQueryOptions = Omit<
  UseQueryOptions<
    SandboxStatusFullResponse,
    Error,
    SandboxStatusFullResponse,
    ReturnType<typeof queryKeys.tasks.sandboxStatus>
  >,
  'queryKey' | 'queryFn'
>

export function useTaskSandboxStatusQuery(taskId: string, options?: TaskSandboxStatusQueryOptions) {
  const {
    enabled = true,
    staleTime = 5000,
    refetchInterval = false,
    ...queryOptions
  } = options ?? {}

  // Polling disabled by default - rely on SSE events for real-time updates
  // Callers can override with refetchInterval if polling is specifically needed

  return useQuery({
    queryKey: queryKeys.tasks.sandboxStatus(taskId),
    queryFn: () => unwrapResponse<SandboxStatusFullResponse>(sandboxStatusHandler(taskId)),
    enabled: enabled && !!taskId,
    staleTime,
    refetchInterval,
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
    ...queryOptions,
  })
}

import type { GitDiffResponse, Task, WorkspaceStatusResponse } from '@moatless/types'
import { type UseQueryOptions, useQuery } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

type TaskDetailQueryOptions = Omit<
  UseQueryOptions<Task, Error, Task, ReturnType<typeof queryKeys.tasks.detail>>,
  'queryKey' | 'queryFn'
>

export function useTaskDetailQuery(taskId: string, options?: TaskDetailQueryOptions) {
  const { enabled = true, staleTime = 30_000, ...queryOptions } = options ?? {}

  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: enabled && !!taskId,
    staleTime,
    refetchOnMount: 'always',
    refetchOnWindowFocus: true,
    ...queryOptions,
  })
}

type TaskGitDiffQueryOptions = Omit<
  UseQueryOptions<
    GitDiffResponse,
    Error,
    GitDiffResponse,
    ReturnType<typeof queryKeys.tasks.gitDiff>
  >,
  'queryKey' | 'queryFn'
>

export function useTaskGitDiffQuery(taskId: string, options?: TaskGitDiffQueryOptions) {
  const {
    enabled = true,
    staleTime = 10 * 60 * 1000,
    retry = false,
    ...queryOptions
  } = options ?? {}

  return useQuery({
    queryKey: queryKeys.tasks.gitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: enabled && !!taskId,
    staleTime,
    retry,
    ...queryOptions,
  })
}

type TaskWorkspaceStatusQueryOptions = Omit<
  UseQueryOptions<
    WorkspaceStatusResponse,
    Error,
    WorkspaceStatusResponse,
    ReturnType<typeof queryKeys.tasks.workspaceStatus>
  >,
  'queryKey' | 'queryFn'
>

export function useTaskWorkspaceStatusQuery(
  taskId: string,
  options?: TaskWorkspaceStatusQueryOptions,
) {
  const { enabled = true, staleTime = 2000, refetchInterval, ...queryOptions } = options ?? {}

  const defaultRefetchInterval: NonNullable<TaskWorkspaceStatusQueryOptions['refetchInterval']> = (
    query,
  ) => {
    const data = query.state.data as WorkspaceStatusResponse | undefined
    const envStatus = data?.environmentStatus
    const agentState = data?.agentState

    if (envStatus === 'initializing' || envStatus === 'cleaning' || agentState === 'running') {
      return 3000
    }
    return false
  }

  return useQuery({
    queryKey: queryKeys.tasks.workspaceStatus(taskId),
    queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
    enabled: enabled && !!taskId,
    staleTime,
    refetchInterval: refetchInterval ?? defaultRefetchInterval,
    ...queryOptions,
  })
}

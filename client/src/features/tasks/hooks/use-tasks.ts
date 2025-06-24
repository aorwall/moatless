import type { FileSearchResponse } from '@shared/types/environmenttypes/environment'
import type { Event } from '@shared/types/event'
import type { GitDiffResponse, TaskResponse } from '@shared/types/task'
import type { ServerStatus } from '@shared/types/workspace'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent'
import type { WorkspaceStatusResponse } from '@/types/task'

/**
 * Task Query Invalidation Strategy:
 *
 * - queryKeys.tasks() - Invalidates all task queries (list + all individual tasks)
 * - queryKeys.task(taskId) - Invalidates all data for a specific task (detail, messages, events, etc.)
 *
 * For mutations:
 * - Always invalidate queryKeys.task(taskId) to refresh all task data
 * - Also invalidate queryKeys.tasks() to update the task list
 */

// Query hooks
export function useTasksList(
  params?: {
    activeOnly?: boolean
    limit?: number
    offset?: number
  },
  options?: Omit<UseQueryOptions<Array<TaskResponse>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasksList(params),
    queryFn: () => tasksApi.listTasks(params),
    ...options,
  })
}

export function useTask(
  taskId: string,
  options?: Omit<UseQueryOptions<TaskResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.task(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: !!taskId,
    ...options,
  })
}

// Mutation hooks
export function useCreateTask(
  options?: UseMutationOptions<AgentTriggerResponse, Error, AgentTriggerRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: AgentTriggerRequest) => tasksApi.createTask(request),
    onSuccess: (data, variables, context) => {
      // Invalidate tasks list to refresh with new task
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks() })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useSendMessage(
  options?: UseMutationOptions<AgentTriggerResponse, Error, { taskId: string; message: string }>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ taskId, message }) => tasksApi.sendMessage(taskId, message),
    onSuccess: async (data, variables, context) => {
      // Invalidate all data for this specific task (detail, messages, events, etc.)
      await queryClient.invalidateQueries({
        queryKey: queryKeys.task(variables.taskId),
      })
      // Also invalidate the tasks list to update metadata like last_message_at
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks(),
      })
      // Invalidate workspace status since sending a message may trigger workspace activity
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskWorkspaceStatus(variables.taskId),
      })
      // Force refetch the task data immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.task(variables.taskId),
      })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useStopAgent(
  options?: UseMutationOptions<{ status: string; taskId: string }, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.stopAgent(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate all data for this specific task
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      // Also invalidate the tasks list to update the status
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks(),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useTaskEvents(
  taskId: string,
  params?: {
    limit?: number
    offset?: number
  },
  options?: Omit<UseQueryOptions<Array<Event>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskEvents(taskId, params),
    queryFn: () => tasksApi.getTaskEvents(taskId, params),
    enabled: !!taskId,
    ...options,
  })
}

export function useTaskGitDiff(
  taskId: string,
  options?: Omit<UseQueryOptions<GitDiffResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskGitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useTaskFileSearch(
  taskId: string,
  query: string,
  options?: Omit<UseQueryOptions<FileSearchResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskFileSearch(taskId, query),
    queryFn: () => tasksApi.searchTaskFiles(taskId, { query }),
    enabled: !!taskId && query.length >= 3,
    staleTime: 30 * 1000, // 30 seconds
    ...options,
  })
}

export function useTaskWorkspaceStatus(
  taskId: string,
  options?: Omit<UseQueryOptions<WorkspaceStatusResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskWorkspaceStatus(taskId),
    queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
    enabled: !!taskId,
    refetchInterval: (query) => {
      // Poll more frequently when workspace is in a transitional state
      const data = query.state.data
      if (data?.status === 'setting_up' || data?.status === 'running') {
        return 1000 // 1 second
      }
      return 30000 // 30 seconds for idle/not_setup states
    },
    ...options,
  })
}

// Server management hooks
export function useServerStatus(
  taskId: string,
  options?: Omit<UseQueryOptions<ServerStatus>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskServerStatus(taskId),
    queryFn: () => tasksApi.getServerStatus(taskId),
    enabled: !!taskId,
    refetchInterval: (query) => {
      // Poll more frequently when server is running
      const data = query.state.data
      if (data?.running) {
        return 5000 // 5 seconds
      }
      return false
    },
    ...options,
  })
}

export function useStartServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.startServer(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate server status to refresh with new state
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Also invalidate task data as server state might affect task metadata
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useStopServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.stopServer(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate server status to refresh with new state
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Also invalidate task data as server state might affect task metadata
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useServerLogs(
  taskId: string,
  options?: Omit<UseQueryOptions<{ logs: string; lines: number }>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskServerLogs(taskId),
    queryFn: () => tasksApi.getServerLogs(taskId),
    enabled: !!taskId,
    refetchInterval: 2000, // Poll every 2 seconds
    ...options,
  })
}

export function useMergeTask(
  options?: UseMutationOptions<{ status: string; message: string }, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.mergeTask(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate task data to refresh
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      // Invalidate workspace status to reflect merge
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskWorkspaceStatus(taskId),
      })
      // Invalidate git diff to show updated state
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskGitDiff(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

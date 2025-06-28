import type { CreateTaskRequest, MergeTaskResponse, SimpleTaskResponse } from '@shared/types/api'
import type { FileSearchResponse } from '@shared/types/environment'
import type { Event } from '@shared/types/event'
import type { GitDiffResponse, Task } from '@shared/types/task'
import type { ServerStatus, WorkspaceStatusResponse } from '@shared/types/workspace'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { updateTaskInLists } from '../utils/query-helpers'

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
    status?: 'open' | 'running' | 'error' | 'merged' | 'closed'
    limit?: number
    offset?: number
  },
  options?: Omit<UseQueryOptions<Array<Task>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasksList(params),
    queryFn: () => tasksApi.listTasks(params),
    ...options,
  })
}

export function useTask(
  taskId: string,
  options?: Omit<UseQueryOptions<Task>, 'queryKey' | 'queryFn'>,
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
  options?: UseMutationOptions<SimpleTaskResponse, Error, CreateTaskRequest>,
) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  return useMutation({
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onMutate: async (request) => {
      // Cancel any in-flight task queries
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks() })

      // Create optimistic task
      const tempId = `temp-${Date.now()}`
      const optimisticTask: Task = {
        taskId: tempId,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: request.agentType || 'claude-code',
        environmentId: request.environmentId,
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      // Pre-populate cache with optimistic data
      queryClient.setQueryData<Task>(queryKeys.task(optimisticTask.taskId), optimisticTask)

      return { optimisticTask }
    },
    onSuccess: async (data, variables, context) => {
      // Replace optimistic data with real data
      const fullTask: Task = {
        taskId: data.taskId,
        createdAt: new Date(data.createdAt || new Date()),
        lastMessageAt: new Date(data.updatedAt || new Date()),
        agentType: variables.agentType || 'claude-code',
        environmentId: variables.environmentId,
        status: data.status || 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      queryClient.setQueryData<Task>(queryKeys.task(data.taskId), fullTask)

      // Remove temp task if different ID
      if (context?.optimisticTask.taskId !== data.taskId) {
        queryClient.removeQueries({
          queryKey: queryKeys.task(context.optimisticTask.taskId),
        })
      }

      // Invalidate lists only (not all task details)
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      })

      // Navigate only after cache is ready
      navigate({ to: `/tasks/${data.taskId}` })

      options?.onSuccess?.(data, variables, context)
    },
    onError: (error, variables, context) => {
      // Rollback optimistic update
      if (context?.optimisticTask) {
        queryClient.removeQueries({
          queryKey: queryKeys.task(context.optimisticTask.taskId),
        })
      }
      options?.onError?.(error, variables, context)
    },
    ...options,
  })
}

export function useSendMessage(
  options?: UseMutationOptions<SimpleTaskResponse, Error, { taskId: string; message: string }>,
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
  options?: UseMutationOptions<{ status: string; task_id: string }, Error, string>,
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
  type?: string,
  options?: Omit<UseQueryOptions<Array<Event>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskEvents(taskId, { type }),
    queryFn: () => tasksApi.getTaskEvents(taskId, type),
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
    onSuccess: async (data, taskId, context) => {
      // Invalidate server status to refresh with new state
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Also invalidate task data as server state might affect task metadata
      await queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
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

export function useRestartServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.restartServer(taskId),
    onSuccess: async (data, taskId, context) => {
      // Invalidate server status to refresh with new state
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Also invalidate task data as server state might affect task metadata
      await queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
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
  options?: UseMutationOptions<
    MergeTaskResponse,
    Error,
    { taskId: string; targetBranch?: string; commitMessage?: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ taskId, targetBranch, commitMessage }) =>
      tasksApi.mergeTask(taskId, { targetBranch, commitMessage }),
    onMutate: async ({ taskId }) => {
      // Cancel any in-flight queries for this task
      await queryClient.cancelQueries({ queryKey: queryKeys.task(taskId) })

      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<Task>(queryKeys.task(taskId))

      // Optimistically update the task status to 'merged'
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'merged' as const }
        queryClient.setQueryData<Task>(queryKeys.task(taskId), updatedTask)

        // Update the task in all cached lists
        updateTaskInLists(queryClient, taskId, { status: 'merged' })
      }

      // Return context with snapshot for potential rollback
      return { previousTask }
    },
    onError: (err, variables, context) => {
      // Rollback on error
      if (context?.previousTask) {
        queryClient.setQueryData(queryKeys.task(variables.taskId), context.previousTask)
        updateTaskInLists(queryClient, variables.taskId, context.previousTask)
      }
      options?.onError?.(err, variables, context)
    },
    onSettled: (data, error, variables) => {
      // Always refetch to ensure consistency with server state
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(variables.taskId),
      })
      // Invalidate workspace status to reflect merge
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskWorkspaceStatus(variables.taskId),
      })
      // Invalidate git diff to show updated state
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskGitDiff(variables.taskId),
      })
      // Invalidate task lists that might show this task
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
        exact: false,
      })
    },
    onSuccess: (data, variables, context) => {
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useCloseTask(
  options?: UseMutationOptions<{ status: 'closed'; taskId: string }, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.closeTask(taskId),
    onMutate: async (taskId) => {
      // Cancel any in-flight queries for this task
      await queryClient.cancelQueries({ queryKey: queryKeys.task(taskId) })

      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<Task>(queryKeys.task(taskId))

      // Optimistically update the task status to 'closed'
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'closed' as const }
        queryClient.setQueryData<Task>(queryKeys.task(taskId), updatedTask)

        // Update the task in all cached lists
        updateTaskInLists(queryClient, taskId, { status: 'closed' })
      }

      // Return context with snapshot for potential rollback
      return { previousTask }
    },
    onError: (err, taskId, context) => {
      // Rollback on error
      if (context?.previousTask) {
        queryClient.setQueryData(queryKeys.task(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, taskId, context)
    },
    onSettled: (data, error, taskId) => {
      // Always refetch to ensure consistency with server state
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      // Invalidate workspace status
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskWorkspaceStatus(taskId),
      })
      // Invalidate server status
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Invalidate task lists that might show this task
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
        exact: false,
      })
    },
    onSuccess: (data, taskId, context) => {
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useDeleteTask(options?: UseMutationOptions<{ status: string }, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.deleteTask(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate all task queries to refresh the list
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks() })
      // Also invalidate the specific task
      queryClient.invalidateQueries({ queryKey: queryKeys.task(taskId) })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

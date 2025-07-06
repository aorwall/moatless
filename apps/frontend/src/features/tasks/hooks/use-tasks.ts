import type {
  CreateTaskRequest,
  MergeTaskResponse,
  SimpleTaskResponse,
  WorkspaceStatusResponse,
} from '@moatless/shared/types/api'
import type { Event } from '@moatless/shared/types/event'
import type { FileSearchResponse } from '@moatless/shared/types/repository'
import type { GitDiffResponse, Task } from '@moatless/shared/types/task'
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
    onSuccess: async (data, variables, context) => {
      // Pre-populate cache with real task data so the page loads immediately
      const fullTask: Task = {
        taskId: data.taskId,
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: variables.agentType || 'claude-code',
        repositoryId: variables.repositoryId,
        status: (data.status as Task['status']) || 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      queryClient.setQueryData<Task>(queryKeys.task(data.taskId), fullTask)

      // Invalidate task lists to show the new task
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.lists(),
      })

      // Navigate to the real task URL now that we have the ID
      navigate({ to: `/tasks/${data.taskId}` })

      options?.onSuccess?.(data, variables, context)
    },
    onError: (error, variables, context) => {
      options?.onError?.(error, variables, context)
    },
    ...options,
  })
}

export function useSendMessage(
  options?: UseMutationOptions<
    SimpleTaskResponse,
    Error,
    {
      taskId: string
      message: string
      resume?: boolean
      model?: string
      fileIds?: string[]
      filePaths?: string[]
      commandName?: string
    }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ taskId, message, resume, model, fileIds, filePaths, commandName }) =>
      tasksApi.sendMessage(taskId, message, resume, model, fileIds, filePaths, commandName),
    onSuccess: async (data, variables, context) => {
      // Only invalidate task lists - events will handle task-specific invalidations
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      })
      // Immediately refetch task data for instant feedback
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
      // Only invalidate task lists - STOP event will handle task-specific invalidations
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
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
  repositoryId: string,
  options?: Omit<UseQueryOptions<FileSearchResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskFileSearch(taskId, query),
    queryFn: () => tasksApi.searchTaskFiles(taskId, { query, repositoryId }),
    enabled: !!taskId && !!repositoryId && query.length >= 3,
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
    refetchInterval: (_query) => {
      return false
    },
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

  return useMutation<
    MergeTaskResponse,
    Error,
    { taskId: string; targetBranch?: string; commitMessage?: string }
  >({
    mutationFn: ({ taskId, targetBranch, commitMessage }) =>
      tasksApi.mergeTask(taskId, { targetBranch, commitMessage }),
    onMutate: async (variables) => {
      // Cancel any in-flight queries for this task
      await queryClient.cancelQueries({ queryKey: queryKeys.task(variables.taskId) })

      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<Task>(queryKeys.task(variables.taskId))

      // Optimistically update the task status to 'merged'
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'merged' as const }
        queryClient.setQueryData<Task>(queryKeys.task(variables.taskId), updatedTask)

        // Update the task in all cached lists
        updateTaskInLists(queryClient, variables.taskId, { status: 'merged' })
      }

      // Return context with snapshot for potential rollback
      return { previousTask }
    },
    onError: (err, variables, context) => {
      // Rollback on error
      if (context && typeof context === 'object' && 'previousTask' in context && context.previousTask) {
        queryClient.setQueryData(queryKeys.task(variables.taskId), context.previousTask)
        updateTaskInLists(queryClient, variables.taskId, context.previousTask)
      }
      options?.onError?.(err, variables, context)
    },
    onSettled: (_data, _error, _variables) => {
      // Only invalidate task lists - events will handle the rest
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

  return useMutation<{ status: 'closed'; taskId: string }, Error, string>({
    mutationFn: (taskId: string) => tasksApi.closeTask(taskId),
    onMutate: async (taskId) => {

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
      if (context && typeof context === 'object' && 'previousTask' in context && context.previousTask) {
        queryClient.setQueryData(queryKeys.task(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, taskId, context)
    },
    onSettled: async (_data, _error, _taskId) => {
      // Only force refetch task lists - events will handle task-specific updates
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasksList(),
        exact: false,
        type: 'active',
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
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all })
      // Also invalidate the specific task
      queryClient.invalidateQueries({ queryKey: queryKeys.task(taskId) })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

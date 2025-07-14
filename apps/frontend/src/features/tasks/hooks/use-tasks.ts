import type {
  CreateTaskRequest,
  MergeTaskResponse,
  SimpleTaskResponse,
  TaskCommandsResponse,
  TaskListResponse,
} from '@moatless/types/types/api'
import type { Event } from '@moatless/types/types/event'
import type { FileSearchResponse } from '@moatless/types/types/repository'
import type { GitDiffResponse, Task } from '@moatless/types/types/task'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
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
  options?: Omit<UseQueryOptions<TaskListResponse>, 'queryKey' | 'queryFn'>,
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

  return useMutation({
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onSuccess: async (data, variables, context) => {
      // Add the real task data to cache and lists
      if (data.task) {
        // Set task detail cache
        queryClient.setQueryData(queryKeys.task(data.taskId), data.task)

        // Add to task lists optimistically
        updateTaskInLists(queryClient, data.taskId, data.task, true)
      } else {
        // Fallback: if no task data returned, invalidate to refetch
        await queryClient.invalidateQueries({
          queryKey: queryKeys.tasks.lists(),
        })
      }

      options?.onSuccess?.(data, variables, context)
    },
    onError: (error, variables, context) => {
      options?.onError?.(error, variables, context)
    },
    ...options,
  })
}

export function useSendMessage(
  taskId: string,
  options?: UseMutationOptions<
    SimpleTaskResponse,
    Error,
    {
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
    mutationKey: ['tasks', taskId, 'sendMessage'],
    mutationFn: ({ message, resume, model, fileIds, filePaths, commandName }) =>
      tasksApi.sendMessage(taskId, message, resume, model, fileIds, filePaths, commandName),
    onSuccess: async (data, variables, context) => {
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      })
      await queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useStopAgent(
  taskId: string,
  options?: UseMutationOptions<{ status: string; taskId: string }, Error, void>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationKey: ['tasks', taskId, 'stop'],
    mutationFn: () => tasksApi.stopAgent(taskId),
    onSuccess: (data, _, context) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasksList(),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.task(taskId),
      })
      options?.onSuccess?.(data, undefined, context)
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

export function useTaskCommands(
  taskId: string,
  options?: Omit<UseQueryOptions<TaskCommandsResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskCommands(taskId),
    queryFn: () => tasksApi.getTaskCommands(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useMergeTask(
  taskId: string,
  options?: UseMutationOptions<
    MergeTaskResponse,
    Error,
    { targetBranch?: string; commitMessage?: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<MergeTaskResponse, Error, { targetBranch?: string; commitMessage?: string }>({
    mutationKey: ['tasks', taskId, 'merge'],
    mutationFn: ({ targetBranch, commitMessage }) =>
      tasksApi.mergeTask(taskId, { targetBranch, commitMessage }),
    onMutate: async () => {
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
      if (
        context &&
        typeof context === 'object' &&
        'previousTask' in context &&
        context.previousTask
      ) {
        queryClient.setQueryData(queryKeys.task(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, variables, context)
    },
    onSettled: () => {
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
  taskId: string,
  options?: UseMutationOptions<{ status: 'closed'; taskId: string }, Error, void>,
) {
  const queryClient = useQueryClient()

  return useMutation<{ status: 'closed'; taskId: string }, Error, void>({
    mutationKey: ['tasks', taskId, 'close'],
    mutationFn: () => tasksApi.closeTask(taskId),
    onMutate: async () => {
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
    onError: (err, _, context) => {
      // Rollback on error
      if (
        context &&
        typeof context === 'object' &&
        'previousTask' in context &&
        context.previousTask
      ) {
        queryClient.setQueryData(queryKeys.task(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, undefined, context)
    },
    onSettled: async () => {
      // Only force refetch task lists - events will handle task-specific updates
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasksList(),
        exact: false,
        type: 'active',
      })
    },
    onSuccess: (data, _, context) => {
      options?.onSuccess?.(data, undefined, context)
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

import type { RouteSuccess } from '@moatless/api-contract'
import type { closeTaskRoute } from '@moatless/api-contract/tasks.ts'
import type {
  AgentMessage,
  CreateTaskRequest,
  DeployResponse,
  GitDiffResponse,
  Message,
  SendMessageResponse,
  SimpleTaskResponse,
  Task,
  TaskCommandsResponse,
  TaskListResponse,
} from '@moatless/types'
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
    queryKey: queryKeys.tasks.list(params),
    queryFn: () => tasksApi.listTasks(params),
    ...options,
  })
}

export function useTask(
  taskId: string,
  options?: Omit<UseQueryOptions<Task>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: !!taskId,
    ...options,
  })
}

// Mutation hooks
export function useCreateTask(
  options?: UseMutationOptions<SimpleTaskResponse, Error, CreateTaskRequest>,
): ReturnType<typeof useMutation<SimpleTaskResponse, Error, CreateTaskRequest>> {
  const queryClient = useQueryClient()

  return useMutation<SimpleTaskResponse, Error, CreateTaskRequest>({
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onSuccess: async (data, variables) => {
      // Add the real task data to cache and lists
      const response = data as SimpleTaskResponse
      if (response.task) {
        // Set task detail cache
        queryClient.setQueryData(queryKeys.tasks.detail(response.taskId), response.task)

        // Add to task lists with real data
        updateTaskInLists(queryClient, response.taskId, response.task, true)

        // Add initial message to messages cache if message was provided
        if (variables.message && response.taskId) {
          const initialMessage: Message = {
            id: `initial-${response.taskId}`,
            role: 'user',
            content: variables.message,
            timestamp: new Date(),
          }

          queryClient.setQueryData<Message[]>(queryKeys.tasks.messages(response.taskId), [
            initialMessage,
          ])
        }
      } else {
        // Fallback: if no task data returned, invalidate to refetch
        await queryClient.invalidateQueries({
          queryKey: queryKeys.tasks.lists(),
        })
      }

      options?.onSuccess?.(data, variables, undefined)
    },
    onError: (error, variables) => {
      options?.onError?.(error, variables, undefined)
    },
    ...options,
  })
}

export function useSendMessage(
  taskId: string,
  options?: UseMutationOptions<
    SendMessageResponse,
    Error,
    {
      message: string
      resume?: boolean
      model?: string
      fileIds?: string[]
      filePaths?: string[]
      commandName?: string
      agentMode?: string
    },
    { previousMessages: Message[] }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    SendMessageResponse,
    Error,
    {
      message: string
      resume?: boolean
      model?: string
      fileIds?: string[]
      filePaths?: string[]
      commandName?: string
      agentMode?: string
    },
    { previousMessages: Message[] }
  >({
    mutationKey: ['tasks', taskId, 'sendMessage'],
    mutationFn: ({ message, resume, model, fileIds, filePaths, commandName, agentMode }) =>
      tasksApi.sendMessage(
        taskId,
        message,
        resume,
        model,
        fileIds,
        filePaths,
        commandName,
        agentMode,
      ),
    onMutate: async (variables) => {
      // Cancel any outgoing refetches (so they don't overwrite our optimistic update)
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.messages(taskId),
      })

      // Get current messages
      const previousMessages =
        queryClient.getQueryData<Message[]>(queryKeys.tasks.messages(taskId)) || []

      // Create optimistic message
      const optimisticMessage: Message = {
        id: `temp-message-${Date.now()}`,
        role: 'user',
        content: variables.message,
        timestamp: new Date(),
      }

      // Update cache with new message
      queryClient.setQueryData<Message[]>(queryKeys.tasks.messages(taskId), [
        ...previousMessages,
        optimisticMessage,
      ])

      // Return context for rollback
      return { previousMessages }
    },
    onSuccess: async (data, variables, context) => {
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      // Don't invalidate messages - they will be updated via SSE events when AI responds
      options?.onSuccess?.(data, variables, context)
    },
    onError: (error, variables, context) => {
      // Rollback optimistic update on error
      if (context?.previousMessages) {
        queryClient.setQueryData(queryKeys.tasks.messages(taskId), context.previousMessages)
      }
      options?.onError?.(error, variables, context)
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
        queryKey: queryKeys.tasks.lists(),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, undefined, context)
    },
    ...options,
  })
}

export function useTaskMessages(
  taskId: string,
  options?: Omit<UseQueryOptions<Array<AgentMessage>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.messages(taskId),
    queryFn: () => tasksApi.getTaskMessages(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useTaskGitDiff(
  taskId: string,
  options?: Omit<UseQueryOptions<GitDiffResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.gitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useTaskCommands(
  taskId: string,
  options?: Omit<UseQueryOptions<TaskCommandsResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.commands(taskId),
    queryFn: () => tasksApi.getTaskCommands(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useCloseTask(
  taskId: string,
  options?: UseMutationOptions<RouteSuccess<typeof closeTaskRoute>, Error, void>,
) {
  const queryClient = useQueryClient()

  return useMutation<RouteSuccess<typeof closeTaskRoute>, Error, void>({
    mutationKey: ['tasks', taskId, 'close'],
    mutationFn: () => tasksApi.closeTask(taskId),
    onMutate: async () => {
      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<Task>(queryKeys.tasks.detail(taskId))

      // Optimistically remove the task from all cached lists
      if (previousTask) {
        // Update task status to 'closed' in detail cache
        const updatedTask = { ...previousTask, status: 'closed' as const }
        queryClient.setQueryData<Task>(queryKeys.tasks.detail(taskId), updatedTask)

        // Remove the task from all cached lists for immediate UI feedback
        updateTaskInLists(queryClient, taskId, null)
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
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, undefined, context)
    },
    onSettled: async () => {
      // Only force refetch task lists - events will handle task-specific updates
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.lists(),
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
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useDeployTask(
  taskId: string,
  options?: UseMutationOptions<DeployResponse, Error, void>,
) {
  const queryClient = useQueryClient()

  return useMutation<DeployResponse, Error, void>({
    mutationKey: ['tasks', taskId, 'deploy'],
    mutationFn: () => tasksApi.deployTask(taskId),
    onSuccess: (data, _, context) => {
      // Invalidate task data to refresh any status changes
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, undefined, context)
    },
    ...options,
  })
}

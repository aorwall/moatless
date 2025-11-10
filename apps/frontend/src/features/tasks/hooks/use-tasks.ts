import type { RouteSuccess } from '@moatless/api-contract'
import type { closeTaskRoute } from '@moatless/api-contract/tasks.ts'
import type {
  CreateTaskRequest,
  CreateVibeTaskRequest,
  DeployResponse,
  GitDiffResponse,
  Message,
  MessageContext,
  SendMessageResponse,
  SimpleTaskResponse,
  Task,
  TaskCommandsResponse,
  TaskListResponse,
} from '@moatless/types'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { updateTaskInLists } from '@/features/tasks'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

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
    ...options,
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
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

      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    onError: (error, variables, onMutateResult, mutationContext) => {
      options?.onError?.(error, variables, onMutateResult, mutationContext)
    },
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
      commandName?: string
      agentMode?: string
      contexts?: MessageContext[]
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
      commandName?: string
      agentMode?: string
      contexts?: MessageContext[]
    },
    { previousMessages: Message[] }
  >({
    ...options,
    mutationKey: ['tasks', taskId, 'sendMessage'],
    mutationFn: ({ message, resume, model, fileIds, commandName, agentMode, contexts }) =>
      tasksApi.sendMessage(
        taskId,
        message,
        resume,
        model,
        fileIds,
        commandName,
        agentMode,
        contexts,
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
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      // Don't invalidate messages - they will be updated via SSE events when AI responds
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    onError: (error, variables, onMutateResult, mutationContext) => {
      // Rollback optimistic update on error
      if (onMutateResult?.previousMessages) {
        queryClient.setQueryData(queryKeys.tasks.messages(taskId), onMutateResult.previousMessages)
      }
      options?.onError?.(error, variables, onMutateResult, mutationContext)
    },
  })
}

export function useStopAgent(
  taskId: string,
  options?: UseMutationOptions<{ status: string; taskId: string }, Error>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    ...options,
    mutationKey: ['tasks', taskId, 'stop'],
    mutationFn: () => tasksApi.stopAgent(taskId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.lists(),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useTaskMessages(
  taskId: string,
  options?: Omit<UseQueryOptions<Message[]>, 'queryKey' | 'queryFn'>,
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
  options?: UseMutationOptions<RouteSuccess<typeof closeTaskRoute>, Error>,
) {
  const queryClient = useQueryClient()

  return useMutation<RouteSuccess<typeof closeTaskRoute>, Error, void>({
    ...options,
    mutationKey: ['tasks', taskId, 'close'],
    mutationFn: () => tasksApi.closeTask(taskId),
    onMutate: () => {
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
    onError: (err, variables, onMutateResult, mutationContext) => {
      // Rollback on error
      if (
        onMutateResult &&
        typeof onMutateResult === 'object' &&
        'previousTask' in onMutateResult &&
        onMutateResult.previousTask
      ) {
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), onMutateResult.previousTask)
        updateTaskInLists(queryClient, taskId, onMutateResult.previousTask)
      }
      options?.onError?.(err, variables, onMutateResult, mutationContext)
    },
    onSettled: async () => {
      // Only force refetch task lists - events will handle task-specific updates
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.lists(),
        exact: false,
        type: 'active',
      })
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useDeleteTask(options?: UseMutationOptions<{ status: string }, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    ...options,
    mutationFn: (taskId: string) => tasksApi.deleteTask(taskId),
    onSuccess: (data, taskId, onMutateResult, mutationContext) => {
      // Invalidate all task queries to refresh the list
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all })
      // Also invalidate the specific task
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, taskId, onMutateResult, mutationContext)
    },
  })
}

export function useDeployTask(taskId: string, options?: UseMutationOptions<DeployResponse, Error>) {
  const queryClient = useQueryClient()

  return useMutation<DeployResponse, Error, void>({
    ...options,
    mutationKey: ['tasks', taskId, 'deploy'],
    mutationFn: () => tasksApi.deployTask(taskId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate task data to refresh any status changes
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

type CreateVibeTaskResponse = { taskId: string; status: string }

/**
 * Create a vibe task (task from a template repository)
 * This creates a new local repository from the template and then creates a task with it
 */
export function useCreateVibeTask(
  options?: UseMutationOptions<CreateVibeTaskResponse, Error, CreateVibeTaskRequest, void>,
) {
  const queryClient = useQueryClient()
  const { onSuccess: externalOnSuccess, ...restOptions } = options ?? {}

  return useMutation<CreateVibeTaskResponse, Error, CreateVibeTaskRequest, void>({
    mutationFn: async (request: CreateVibeTaskRequest) => {
      return await tasksApi.createVibeTask(request)
    },
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      // Add initial message to messages cache if message was provided
      if (variables.message && data.taskId) {
        const initialMessage: Message = {
          id: `initial-${data.taskId}`,
          role: 'user',
          content: variables.message,
          timestamp: new Date(),
        }

        queryClient.setQueryData<Message[]>(queryKeys.tasks.messages(data.taskId), [initialMessage])
      }

      // The vibe task endpoint only returns taskId and status, not the full task
      // So we need to invalidate the task lists to refetch with the real data
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.lists(),
      })

      externalOnSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...restOptions,
  })
}

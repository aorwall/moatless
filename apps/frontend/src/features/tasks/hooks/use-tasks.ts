import type {
  CreateTaskRequest,
  Event,
  GitDiffResponse,
  MergeTaskResponse,
  Message,
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
    onMutate: async (variables) => {
      // Cancel any outgoing refetches (so they don't overwrite our optimistic update)
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.lists() })

      // Create optimistic task
      const optimisticTaskId = `temp-${Date.now()}`
      const now = new Date()
      const optimisticTask: Task = {
        taskId: optimisticTaskId,
        createdAt: now,
        lastMessageAt: now,
        agentType: variables.agentType,
        repositoryId: variables.repositoryId,
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
        templateId: variables.templateId,
      }

      // Optimistically add task to all cached lists
      updateTaskInLists(queryClient, optimisticTaskId, optimisticTask, true)

      // Return context for potential rollback
      return { optimisticTaskId }
    },
    onSuccess: async (data, variables, context) => {
      // Remove optimistic task if we created one
      if (context?.optimisticTaskId) {
        updateTaskInLists(queryClient, context.optimisticTaskId, null)
      }

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

      options?.onSuccess?.(data, variables, context)
    },
    onError: (error, variables, context) => {
      // Remove optimistic task on error
      if (context?.optimisticTaskId) {
        updateTaskInLists(queryClient, context.optimisticTaskId, null)
      }

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
      agentMode?: string
    }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    SimpleTaskResponse,
    Error,
    {
      message: string
      resume?: boolean
      model?: string
      fileIds?: string[]
      filePaths?: string[]
      commandName?: string
      agentMode?: string
    }
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
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.messages(taskId) })

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
        queryKey: queryKeys.tasks.lists(),
      })
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

export function useTaskEvents(
  taskId: string,
  type?: string,
  options?: Omit<UseQueryOptions<Array<Event>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: type
      ? ([...queryKeys.tasks.events(taskId), { type }] as const)
      : queryKeys.tasks.events(taskId),
    queryFn: () => tasksApi.getTaskEvents(taskId, type),
    enabled: !!taskId,
    ...options,
  })
}

export function useTaskMessages(
  taskId: string,
  options?: Omit<UseQueryOptions<Array<Message>>, 'queryKey' | 'queryFn'>,
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
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.detail(taskId) })

      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<Task>(queryKeys.tasks.detail(taskId))

      // Optimistically update the task status to 'merged'
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'merged' as const }
        queryClient.setQueryData<Task>(queryKeys.tasks.detail(taskId), updatedTask)

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
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(err, variables, context)
    },
    onSettled: () => {
      // Only invalidate task lists - events will handle the rest
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.lists(),
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
      const previousTask = queryClient.getQueryData<Task>(queryKeys.tasks.detail(taskId))

      // Optimistically update the task status to 'closed'
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'closed' as const }
        queryClient.setQueryData<Task>(queryKeys.tasks.detail(taskId), updatedTask)

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
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(taskId) })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

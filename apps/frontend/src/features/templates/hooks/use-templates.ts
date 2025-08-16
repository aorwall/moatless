import type { CreateVibeTaskRequest, Message, Task } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { updateTaskInLists } from '@/features/tasks/utils/query-helpers'
import { type ApiError, apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { RepositoryTemplate } from '../types'

// For now, we'll use hardcoded templates since there's no backend endpoint yet
// This matches the template defined in the backend service
const REPOSITORY_TEMPLATES: RepositoryTemplate[] = [
  {
    id: 'hello-world-ui',
    name: 'Basic Hello Website',
    description: 'A simple website template with HTML, CSS, and JavaScript',
    gitUrl: 'https://github.com/aorwall/hello-world-ui.git',
    setupCommand: 'bun install',
    previewConfig: {
      serverStartCommand: 'bun run dev',
      serverPort: 3000,
      injectionStrategy: {
        method: 'html-injection',
      },
    },
  },
]

export function useTemplatesList(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.templates.list(),
    queryFn: async () => {
      // For now, return hardcoded templates
      // In the future, this could fetch from an API endpoint
      return { templates: REPOSITORY_TEMPLATES }
    },
    staleTime: 5 * 60 * 1000, // 5 minutes
    enabled: options?.enabled,
  })
}

type CreateVibeTaskResponse = { taskId: string; status: string }

export function useCreateVibeTask(
  options?: UseMutationOptions<
    CreateVibeTaskResponse,
    ApiError,
    CreateVibeTaskRequest,
    { optimisticTaskId: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    CreateVibeTaskResponse,
    ApiError,
    CreateVibeTaskRequest,
    { optimisticTaskId: string }
  >({
    mutationFn: async (request: CreateVibeTaskRequest) => {
      return await apiClient.post<CreateVibeTaskResponse>('/api/v1/tasks/vibe', request)
    },
    onMutate: async (variables) => {
      // Cancel any outgoing refetches (so they don't overwrite our optimistic update)
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.lists() })

      // Create optimistic task
      const optimisticTaskId = `temp-vibe-${Date.now()}`
      const now = new Date()
      const optimisticTask: Task = {
        taskId: optimisticTaskId,
        createdAt: now,
        lastMessageAt: now,
        agentType: variables.agentType,
        repositoryId: '', // Will be created by the backend
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

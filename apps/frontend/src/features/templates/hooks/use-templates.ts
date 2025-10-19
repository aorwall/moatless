import type { CreateVibeTaskRequest, Message, TemplateListResponse } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { type ApiError, apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useTemplatesList(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.templates.list(),
    queryFn: async () => {
      return await apiClient.get<TemplateListResponse>('/api/v1/templates')
    },
    staleTime: 5 * 60 * 1000, // 5 minutes
    enabled: options?.enabled,
  })
}

type CreateVibeTaskResponse = { taskId: string; status: string }

export function useCreateVibeTask(
  options?: UseMutationOptions<CreateVibeTaskResponse, ApiError, CreateVibeTaskRequest, void>,
) {
  const queryClient = useQueryClient()
  const { onSuccess: externalOnSuccess, ...restOptions } = options ?? {}

  return useMutation<CreateVibeTaskResponse, ApiError, CreateVibeTaskRequest, void>({
    mutationFn: async (request: CreateVibeTaskRequest) => {
      return await apiClient.post<CreateVibeTaskResponse>('/api/v1/tasks/vibe', request)
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

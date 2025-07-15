import type {
  ClaudeCodeSettingsResponse,
  UpdateClaudeCodeSettingsRequest,
} from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

import { claudeCodeApi } from '../api/claude-code'

export const useClaudeCodeSettings = () => {
  return useQuery({
    queryKey: queryKeys.claudeCode.settings(),
    queryFn: claudeCodeApi.getSettings,
  })
}

export const useClaudeCodeTools = () => {
  return useQuery({
    queryKey: queryKeys.claudeCode.tools(),
    queryFn: claudeCodeApi.getAvailableTools,
  })
}

export const useUpdateClaudeCodeSettings = (
  options?: UseMutationOptions<
    ClaudeCodeSettingsResponse,
    ApiError,
    UpdateClaudeCodeSettingsRequest
  >,
) => {
  const queryClient = useQueryClient()

  return useMutation<ClaudeCodeSettingsResponse, ApiError, UpdateClaudeCodeSettingsRequest>({
    mutationFn: (request: UpdateClaudeCodeSettingsRequest) => claudeCodeApi.updateSettings(request),
    onSuccess: (data, variables, context) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.claudeCode.settings(),
      })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export const useDeleteClaudeCodeSettings = (options?: UseMutationOptions<void, ApiError, void>) => {
  const queryClient = useQueryClient()

  return useMutation<void, ApiError, void>({
    mutationFn: claudeCodeApi.deleteSettings,
    onSuccess: (data, variables, context) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.claudeCode.settings(),
      })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

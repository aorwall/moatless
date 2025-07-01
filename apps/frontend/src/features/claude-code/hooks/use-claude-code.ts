import type {
  ClaudeCodeSettingsResponse,
  UpdateClaudeCodeSettingsRequest,
} from '@moatless/shared/types/claude-code'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
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

interface UpdateSettingsOptions {
  onSuccess?: (data: ClaudeCodeSettingsResponse) => void
  onError?: (error: Error) => void
}

export const useUpdateClaudeCodeSettings = (options?: UpdateSettingsOptions) => {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: UpdateClaudeCodeSettingsRequest) => claudeCodeApi.updateSettings(request),
    onSuccess: (data) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.claudeCode.settings(),
      })
      options?.onSuccess?.(data)
    },
    onError: options?.onError,
  })
}

export const useDeleteClaudeCodeSettings = (options?: {
  onSuccess?: () => void
  onError?: (error: Error) => void
}) => {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: claudeCodeApi.deleteSettings,
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.claudeCode.settings(),
      })
      options?.onSuccess?.()
    },
    onError: options?.onError,
  })
}

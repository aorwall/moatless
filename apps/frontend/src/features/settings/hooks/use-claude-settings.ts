import type { ClaudeSettingsResponse, UpdateClaudeSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useClaudeSettings() {
  return useQuery<ClaudeSettingsResponse>({
    queryKey: queryKeys.settings.claude(),
    queryFn: settingsApi.claude.get,
  })
}

export function useUpdateClaudeSettings(
  options?: UseMutationOptions<ClaudeSettingsResponse, ApiError, UpdateClaudeSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<ClaudeSettingsResponse, ApiError, UpdateClaudeSettingsRequest>({
    mutationFn: (request: UpdateClaudeSettingsRequest) => settingsApi.claude.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.claude(), data)
      // Also invalidate the legacy combined settings
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}

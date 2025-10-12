import type { McpSettingsResponse, UpdateMcpSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useMcpSettings() {
  return useQuery<McpSettingsResponse>({
    queryKey: queryKeys.settings.mcp(),
    queryFn: settingsApi.mcp.get,
  })
}

export function useUpdateMcpSettings(
  options?: UseMutationOptions<McpSettingsResponse, ApiError, UpdateMcpSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<McpSettingsResponse, ApiError, UpdateMcpSettingsRequest>({
    mutationFn: (request: UpdateMcpSettingsRequest) => settingsApi.mcp.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.mcp(), data)
      // Also invalidate the legacy combined settings
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}

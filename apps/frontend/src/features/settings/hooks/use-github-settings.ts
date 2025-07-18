import type { GitHubSettingsResponse, UpdateGitHubSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useGitHubSettings() {
  return useQuery<GitHubSettingsResponse>({
    queryKey: queryKeys.settings.github(),
    queryFn: settingsApi.github.get,
  })
}

export function useUpdateGitHubSettings(
  options?: UseMutationOptions<GitHubSettingsResponse, ApiError, UpdateGitHubSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<GitHubSettingsResponse, ApiError, UpdateGitHubSettingsRequest>({
    mutationFn: (request: UpdateGitHubSettingsRequest) => settingsApi.github.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.github(), data)
      // Also invalidate the legacy combined settings
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}

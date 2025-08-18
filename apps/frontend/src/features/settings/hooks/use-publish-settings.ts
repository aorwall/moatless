import type { PublishSettingsResponse, UpdatePublishSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function usePublishSettings() {
  return useQuery<PublishSettingsResponse>({
    queryKey: queryKeys.settings.publish(),
    queryFn: settingsApi.publish.get,
  })
}

export function useUpdatePublishSettings(
  options?: UseMutationOptions<PublishSettingsResponse, ApiError, UpdatePublishSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<PublishSettingsResponse, ApiError, UpdatePublishSettingsRequest>({
    mutationFn: (request: UpdatePublishSettingsRequest) => settingsApi.publish.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.publish(), data)
      // Also invalidate the legacy combined settings
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}

import type { GeneralSettingsResponse, UpdateGeneralSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useGeneralSettings() {
  return useQuery<GeneralSettingsResponse>({
    queryKey: queryKeys.settings.general(),
    queryFn: settingsApi.general.get,
  })
}

export function useUpdateGeneralSettings(
  options?: UseMutationOptions<GeneralSettingsResponse, ApiError, UpdateGeneralSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<GeneralSettingsResponse, ApiError, UpdateGeneralSettingsRequest>({
    mutationFn: (request: UpdateGeneralSettingsRequest) => settingsApi.general.update(request),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.general(), data)
      // Also invalidate the legacy combined settings
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.detail() })
    },
    ...options,
  })
}

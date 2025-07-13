import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { settingsApi } from '../api/settings'

export function useSettings() {
  return useQuery<SettingsResponse>({
    queryKey: ['settings'],
    queryFn: settingsApi.getSettings,
  })
}

export function useUpdateSettings(
  options?: UseMutationOptions<SettingsResponse, ApiError, UpdateSettingsRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<SettingsResponse, ApiError, UpdateSettingsRequest>({
    mutationFn: (request: UpdateSettingsRequest) => settingsApi.updateSettings(request),
    onSuccess: (data) => {
      queryClient.setQueryData(['settings'], data)
    },
    ...options,
  })
}

export function useResetSettings(options?: UseMutationOptions<SettingsResponse, ApiError, void>) {
  const queryClient = useQueryClient()

  return useMutation<SettingsResponse, ApiError, void>({
    mutationFn: () => settingsApi.resetSettings(),
    onSuccess: (data) => {
      queryClient.setQueryData(['settings'], data)
    },
    ...options,
  })
}

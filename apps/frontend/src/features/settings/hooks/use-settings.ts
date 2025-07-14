import type { SettingsResponse, SystemStatusCheck, UpdateSettingsRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useSettings() {
  return useQuery<SettingsResponse>({
    queryKey: queryKeys.settings.detail(),
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
      queryClient.setQueryData(queryKeys.settings.detail(), data)
    },
    ...options,
  })
}

export function useResetSettings(options?: UseMutationOptions<SettingsResponse, ApiError, void>) {
  const queryClient = useQueryClient()

  return useMutation<SettingsResponse, ApiError, void>({
    mutationFn: () => settingsApi.resetSettings(),
    onSuccess: (data) => {
      queryClient.setQueryData(queryKeys.settings.detail(), data)
    },
    ...options,
  })
}

export function useSystemStatus() {
  return useQuery<SystemStatusCheck>({
    queryKey: queryKeys.settings.systemStatus(),
    queryFn: settingsApi.getSystemStatus,
    staleTime: 5 * 60 * 1000, // 5 minutes
    refetchOnWindowFocus: false,
  })
}

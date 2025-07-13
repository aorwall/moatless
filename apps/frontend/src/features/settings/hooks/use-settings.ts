import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/shared'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { settingsApi } from '../api/settings.api'

export function useSettings() {
  return useQuery<SettingsResponse>({
    queryKey: ['settings'],
    queryFn: settingsApi.getSettings,
  })
}

export function useUpdateSettings() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: UpdateSettingsRequest) => settingsApi.updateSettings(request),
    onSuccess: (data) => {
      queryClient.setQueryData(['settings'], data)
    },
  })
}

export function useResetSettings() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: () => settingsApi.resetSettings(),
    onSuccess: (data) => {
      queryClient.setQueryData(['settings'], data)
    },
  })
}

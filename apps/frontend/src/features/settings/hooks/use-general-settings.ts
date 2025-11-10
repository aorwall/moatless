import type { GeneralSettingsResponse } from '@moatless/types'
import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { settingsApi } from '../api/settings'

export function useGeneralSettings() {
  return useQuery<GeneralSettingsResponse>({
    queryKey: queryKeys.settings.general(),
    queryFn: settingsApi.general.get,
  })
}

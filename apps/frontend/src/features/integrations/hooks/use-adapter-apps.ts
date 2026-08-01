import type { AdapterAppSummary, SaveAdapterAppRequest } from '@moatless/api-client'
import type { UseMutationOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { adapterAppsApi } from '@/features/integrations/api/adapter-apps.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useAdapterApps() {
  return useQuery({
    queryKey: queryKeys.integrations.adapterApps(),
    queryFn: () => adapterAppsApi.list(),
  })
}

export function useSaveAdapterApp(
  options?: UseMutationOptions<
    AdapterAppSummary,
    ApiError,
    { adapterKind: string; appKey: string; data: SaveAdapterAppRequest }
  >,
) {
  const queryClient = useQueryClient()
  return useMutation({
    ...options,
    mutationFn: ({ adapterKind, appKey, data }) => adapterAppsApi.save(adapterKind, appKey, data),
    onSuccess: async (...args) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.integrations.adapterApps() })
      await options?.onSuccess?.(...args)
    },
  })
}

export function useDeleteAdapterApp(
  options?: UseMutationOptions<void, ApiError, { adapterKind: string; appKey: string }>,
) {
  const queryClient = useQueryClient()
  return useMutation({
    ...options,
    mutationFn: ({ adapterKind, appKey }) => adapterAppsApi.delete(adapterKind, appKey),
    onSuccess: async (...args) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.integrations.adapterApps() })
      await options?.onSuccess?.(...args)
    },
  })
}

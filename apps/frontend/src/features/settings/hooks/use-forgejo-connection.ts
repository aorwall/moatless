import type {
  ForgejoProviderTokenStatusResponse,
  SaveForgejoProviderTokenRequest,
} from '@moatless/api-client/sandbox/model'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { forgejoConnectionApi } from '@/features/settings/api/forgejo-connection.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * The caller's connection to one Forgejo instance.
 *
 * Disabled until `host` is non-empty: the backend requires it, and the
 * settings screen only knows the host once a repository or an entered URL
 * names one.
 */
export function useForgejoConfig(host: string) {
  return useQuery({
    queryKey: queryKeys.settings.forgejo(host),
    queryFn: () => forgejoConnectionApi.getConfig(host),
    enabled: host.length > 0,
  })
}

/** Disconnect the caller's credential for one Forgejo instance. */
export function useDisconnectForgejo(
  host: string,
  options?: UseMutationOptions<ForgejoProviderTokenStatusResponse, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: () => forgejoConnectionApi.disconnect(host),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.settings.forgejo(host),
      })
    },
    ...options,
  })
}

/** Save a personal access token for the instance named by `baseUrl`. */
export function useSaveForgejoPAT(
  host: string,
  options?: UseMutationOptions<
    ForgejoProviderTokenStatusResponse,
    ApiError,
    SaveForgejoProviderTokenRequest
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: forgejoConnectionApi.savePAT,
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.settings.forgejo(host),
      })
    },
    ...options,
  })
}

/** Drop the personal access token, restoring the OAuth credential beneath it. */
export function useRemoveForgejoPatOverride(
  host: string,
  options?: UseMutationOptions<ForgejoProviderTokenStatusResponse, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: () => forgejoConnectionApi.removePatOverride(host),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.settings.forgejo(host),
      })
    },
    ...options,
  })
}

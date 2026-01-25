import type { PluginInfo, PluginMarketplace } from '@moatless/types'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { pluginsApi } from '@/features/plugins/api/plugins.api'
import { queryKeys } from '@/lib/api/query-keys'

// ============================================================================
// Query Hooks
// ============================================================================

/**
 * Hook to list all available plugins from all marketplaces
 */
export function usePlugins(
  options?: Omit<UseQueryOptions<{ plugins: PluginInfo[] }>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.list(),
    queryFn: () => pluginsApi.listPlugins(),
    staleTime: 5 * 60 * 1000, // Cache for 5 minutes
    ...options,
  })
}

/**
 * Hook to list all plugin marketplaces
 */
export function useMarketplaces(
  options?: Omit<UseQueryOptions<{ marketplaces: PluginMarketplace[] }>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.marketplacesList(),
    queryFn: () => pluginsApi.listMarketplaces(),
    staleTime: 5 * 60 * 1000, // Cache for 5 minutes
    ...options,
  })
}

// ============================================================================
// Mutation Hooks
// ============================================================================

/**
 * Hook to add a new plugin marketplace
 */
export function useAddMarketplace(
  options?: UseMutationOptions<PluginMarketplace, Error, { repoUrl: string }>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ repoUrl }) => pluginsApi.addMarketplace({ repoUrl }),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate plugins and marketplaces lists
      queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

/**
 * Hook to remove a plugin marketplace
 */
export function useRemoveMarketplace(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (marketplaceId) => pluginsApi.removeMarketplace(marketplaceId),
    onSuccess: (data, marketplaceId, onMutateResult, mutationContext) => {
      // Invalidate plugins and marketplaces lists
      queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      options?.onSuccess?.(data, marketplaceId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

/**
 * Hook to sync a plugin marketplace from remote
 */
export function useSyncMarketplace(options?: UseMutationOptions<PluginMarketplace, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (marketplaceId) => pluginsApi.syncMarketplace(marketplaceId),
    onSuccess: (data, marketplaceId, onMutateResult, mutationContext) => {
      // Invalidate plugins and marketplaces lists
      queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      options?.onSuccess?.(data, marketplaceId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

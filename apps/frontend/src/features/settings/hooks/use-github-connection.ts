import type { ExchangeTokenRequest, SavePatRequest } from '@moatless/api-client/sandbox/model'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { githubConnectionApi } from '@/features/settings'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * Get user's GitHub connection configuration
 */
export function useGitHubConfig() {
  return useQuery({
    queryKey: queryKeys.settings.github(),
    queryFn: githubConnectionApi.getConfig,
  })
}

/**
 * Get GitHub OAuth URL
 * This is a query but returns a new URL each time (with new state)
 */
export function useGetGitHubOAuthUrl(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ['github-oauth-url'],
    queryFn: githubConnectionApi.getOAuthUrl,
    enabled: options?.enabled ?? false,
    // Don't cache OAuth URLs - they're single-use with unique state
    gcTime: 0,
    staleTime: 0,
  })
}

/**
 * Exchange OAuth code for token
 */
export function useExchangeGitHubToken(
  options?: UseMutationOptions<
    { success: boolean; message: string },
    ApiError,
    ExchangeTokenRequest
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.exchangeToken,
    onSuccess: () => {
      // Invalidate GitHub config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
    },
    ...options,
  })
}

/**
 * Disconnect GitHub service connection
 */
export function useDisconnectGitHub(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.disconnect,
    onSuccess: () => {
      // Invalidate GitHub config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token
 */
export function useSaveGitHubPAT(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError, SavePatRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.savePAT,
    onSuccess: () => {
      // Invalidate GitHub config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
    },
    ...options,
  })
}

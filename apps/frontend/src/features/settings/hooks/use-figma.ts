import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { FigmaOAuthTokenRequest, FigmaPATRequest } from '@/lib/types/settings'
import { figmaApi } from '../api/figma.api'

/**
 * Get user's Figma configuration
 */
export function useFigmaConfig() {
  return useQuery({
    queryKey: queryKeys.settings.figma(),
    queryFn: figmaApi.getConfig,
  })
}

/**
 * Get Figma OAuth URL
 * This is a query but returns a new URL each time (with new state)
 */
export function useGetFigmaOAuthUrl(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.oauth.figma(),
    queryFn: figmaApi.getOAuthUrl,
    enabled: options?.enabled ?? false,
    // Don't cache OAuth URLs - they're single-use with unique state
    gcTime: 0,
    staleTime: 0,
  })
}

/**
 * Exchange OAuth code for token
 */
export function useExchangeFigmaToken(
  options?: UseMutationOptions<
    { success: boolean; message: string },
    ApiError,
    FigmaOAuthTokenRequest
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: figmaApi.exchangeToken,
    onSuccess: () => {
      // Invalidate Figma config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.figma() })
    },
    ...options,
  })
}

/**
 * Disconnect Figma OAuth
 */
export function useDisconnectFigma(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: figmaApi.disconnect,
    onSuccess: () => {
      // Invalidate Figma config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.figma() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token
 */
export function useSaveFigmaPAT(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError, FigmaPATRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: figmaApi.savePAT,
    onSuccess: () => {
      // Invalidate Figma config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.figma() })
    },
    ...options,
  })
}

import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { claudeConnectionApi } from '@/features/settings/api/claude-connection.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { ClaudePATRequest } from '@/lib/types/settings'

/**
 * Get user's Claude connection configuration
 */
export function useClaudeConfig() {
  return useQuery({
    queryKey: queryKeys.settings.claudeConfig(),
    queryFn: claudeConnectionApi.getConfig,
  })
}

/**
 * Get Claude OAuth URL
 * This is a query but returns a new URL each time (with new state and PKCE challenge)
 */
export function useGetClaudeOAuthUrl(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.settings.claudeOAuthUrl(),
    queryFn: claudeConnectionApi.getOAuthUrl,
    enabled: options?.enabled ?? false,
    // Don't cache OAuth URLs - they're single-use with unique state and PKCE parameters
    gcTime: 0,
    staleTime: 0,
  })
}

/**
 * Disconnect Claude service connection
 */
export function useDisconnectClaude(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: claudeConnectionApi.disconnect,
    onSuccess: () => {
      // Invalidate Claude config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.claudeConfig() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token
 */
export function useSaveClaudePAT(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError, ClaudePATRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: claudeConnectionApi.savePAT,
    onSuccess: () => {
      // Invalidate Claude config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.claudeConfig() })
    },
    ...options,
  })
}

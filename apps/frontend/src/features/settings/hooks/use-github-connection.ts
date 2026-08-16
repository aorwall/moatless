import type {
  GitHubProviderTokenStatusResponse,
  SaveGitHubProviderTokenRequest,
} from '@moatless/api-client/sandbox/model'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { githubConnectionApi } from '@/features/settings/api/github-connection.api'
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
 * Disconnect GitHub service connection
 */
export function useDisconnectGitHub(
  options?: UseMutationOptions<GitHubProviderTokenStatusResponse, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.disconnect,
    onSuccess: () => {
      // Invalidate GitHub config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
      // Refresh the session so the top-bar reconnect indicator reflects the
      // new provider state without requiring a full reload.
      queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token
 */
export function useSaveGitHubPAT(
  options?: UseMutationOptions<
    GitHubProviderTokenStatusResponse,
    ApiError,
    SaveGitHubProviderTokenRequest
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.savePAT,
    onSuccess: () => {
      // Invalidate GitHub config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
      // Refresh the session so the top-bar reconnect indicator reflects the
      // new provider state without requiring a full reload.
      queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() })
    },
    ...options,
  })
}

/**
 * Remove a PAT override and restore the GitHub App user token.
 */
export function useRemoveGitHubPATOverride(
  options?: UseMutationOptions<GitHubProviderTokenStatusResponse, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: githubConnectionApi.removePATOverride,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
      queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() })
    },
    ...options,
  })
}

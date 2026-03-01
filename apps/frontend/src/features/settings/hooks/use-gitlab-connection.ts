import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { gitlabConnectionApi } from '@/features/settings'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { GitLabPATRequest } from '@/lib/types/settings'

/**
 * Get user's GitLab connection configuration
 */
export function useGitLabConfig() {
  return useQuery({
    queryKey: queryKeys.settings.gitlab(),
    queryFn: gitlabConnectionApi.getConfig,
  })
}

/**
 * Disconnect GitLab service connection
 */
export function useDisconnectGitLab(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: gitlabConnectionApi.disconnect,
    onSuccess: () => {
      // Invalidate GitLab config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.gitlab() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token
 */
export function useSaveGitLabPAT(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError, GitLabPATRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: gitlabConnectionApi.savePAT,
    onSuccess: () => {
      // Invalidate GitLab config to refresh connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.gitlab() })
    },
    ...options,
  })
}

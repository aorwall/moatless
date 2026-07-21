import type { GitHubConfigResponse, SavePatRequest } from '@moatless/api-client/sandbox/model'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'

import { adminGithubConnectionApi } from '@/features/settings/api/admin-github-connection.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useAdminGitHubConfig() {
  return useQuery({
    queryKey: queryKeys.admin.github(),
    queryFn: adminGithubConnectionApi.getConfig,
  })
}

/**
 * Invalidate both the admin page's own status and the session — the setup
 * banner reads `setup` off `/auth/me`, and it's an active observer on every
 * authenticated route, so this clears it without a reload.
 */
function useInvalidateOnSuccess() {
  const queryClient = useQueryClient()
  return () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.admin.github() })
    queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() })
    // Repository listing may start succeeding once an org token exists.
    queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() })
  }
}

export function useSaveAdminGitHubPAT(
  options?: UseMutationOptions<GitHubConfigResponse, ApiError, SavePatRequest>,
) {
  const onSuccess = useInvalidateOnSuccess()

  return useMutation({
    mutationFn: adminGithubConnectionApi.savePAT,
    onSuccess,
    ...options,
  })
}

export function useRemoveAdminGitHubPAT(
  options?: UseMutationOptions<GitHubConfigResponse, ApiError, void>,
) {
  const onSuccess = useInvalidateOnSuccess()

  return useMutation({
    mutationFn: adminGithubConnectionApi.remove,
    onSuccess,
    ...options,
  })
}

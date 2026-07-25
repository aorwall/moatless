import type {
  GitHubInstallationBindingRequest,
  GitHubInstallationBindingResponse,
} from '@moatless/api-client/sandbox/model'
import {
  type QueryClient,
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'

import { adminGithubConnectionApi } from '@/features/settings/api/admin-github-connection.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useAdminGitHubApps() {
  return useQuery({
    queryKey: queryKeys.admin.githubApps(),
    queryFn: adminGithubConnectionApi.getApps,
  })
}

export function useAdminGitHubInstallation() {
  return useQuery({
    queryKey: queryKeys.admin.globalGithubInstallation(),
    queryFn: () => adminGithubConnectionApi.getGlobalInstallation(),
  })
}

function useInvalidateGitHubInstallation() {
  const queryClient = useQueryClient()
  return () => invalidateGitHubInstallationCaches(queryClient)
}

export async function invalidateGitHubInstallationCaches(queryClient: QueryClient) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.admin.github() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all }),
  ])
}

export function useSaveAdminGitHubInstallation(
  options?: UseMutationOptions<
    GitHubInstallationBindingResponse,
    ApiError,
    GitHubInstallationBindingRequest
  >,
) {
  const invalidate = useInvalidateGitHubInstallation()
  return useMutation({
    ...options,
    mutationFn: (request) => adminGithubConnectionApi.saveGlobalInstallation(request),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

export function useRemoveAdminGitHubInstallation(
  options?: UseMutationOptions<GitHubInstallationBindingResponse, ApiError, void>,
) {
  const invalidate = useInvalidateGitHubInstallation()
  return useMutation({
    ...options,
    mutationFn: () => adminGithubConnectionApi.removeGlobalInstallation(),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

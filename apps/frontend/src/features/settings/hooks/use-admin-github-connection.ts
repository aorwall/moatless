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

export function useAdminGitHubInstallation(teamId?: string) {
  return useQuery({
    queryKey: teamId
      ? queryKeys.admin.teamGithubInstallation(teamId)
      : queryKeys.admin.globalGithubInstallation(),
    queryFn: () =>
      teamId
        ? adminGithubConnectionApi.getTeamInstallation(teamId)
        : adminGithubConnectionApi.getGlobalInstallation(),
  })
}

function useInvalidateGitHubInstallation(teamId?: string) {
  const queryClient = useQueryClient()
  return () => invalidateGitHubInstallationCaches(queryClient, teamId)
}

export async function invalidateGitHubInstallationCaches(
  queryClient: QueryClient,
  teamId?: string,
) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.admin.github() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all }),
  ])
  if (teamId) {
    await queryClient.invalidateQueries({
      queryKey: queryKeys.admin.teamGithubInstallation(teamId),
    })
  }
}

export function useSaveAdminGitHubInstallation(
  teamId?: string,
  options?: UseMutationOptions<
    GitHubInstallationBindingResponse,
    ApiError,
    GitHubInstallationBindingRequest
  >,
) {
  const invalidate = useInvalidateGitHubInstallation(teamId)
  return useMutation({
    ...options,
    mutationFn: (request) =>
      teamId
        ? adminGithubConnectionApi.saveTeamInstallation(teamId, request)
        : adminGithubConnectionApi.saveGlobalInstallation(request),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

export function useRemoveAdminGitHubInstallation(
  teamId?: string,
  options?: UseMutationOptions<GitHubInstallationBindingResponse, ApiError, void>,
) {
  const invalidate = useInvalidateGitHubInstallation(teamId)
  return useMutation({
    ...options,
    mutationFn: () =>
      teamId
        ? adminGithubConnectionApi.removeTeamInstallation(teamId)
        : adminGithubConnectionApi.removeGlobalInstallation(),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

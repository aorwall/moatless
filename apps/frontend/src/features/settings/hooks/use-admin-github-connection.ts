import type {
  CreateUserResponse,
  GitHubAppRegistrationResponse,
  RegisterGitHubAppRequest,
  RemoveGitHubAppResponse,
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

function useInvalidateGitHubApps() {
  const queryClient = useQueryClient()
  return () => invalidateGitHubAppCaches(queryClient)
}

/// Registering, rotating or removing an App changes what every repository
/// listing and every Task setup check can reach, so those caches go too.
export async function invalidateGitHubAppCaches(queryClient: QueryClient) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.admin.github() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.auth.session() }),
    queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all }),
  ])
}

export function useRegisterAdminGitHubApp(
  options?: UseMutationOptions<GitHubAppRegistrationResponse, ApiError, RegisterGitHubAppRequest>,
) {
  const invalidate = useInvalidateGitHubApps()
  return useMutation({
    ...options,
    mutationFn: (request) => adminGithubConnectionApi.registerApp(request),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

export function useRotateAdminGitHubAppKey(
  options?: UseMutationOptions<
    GitHubAppRegistrationResponse,
    ApiError,
    { githubAppKey: string; privateKeyPem: string }
  >,
) {
  const invalidate = useInvalidateGitHubApps()
  return useMutation({
    ...options,
    mutationFn: ({ githubAppKey, privateKeyPem }) =>
      adminGithubConnectionApi.rotateAppKey(githubAppKey, { privateKeyPem }),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

export function useRemoveAdminGitHubApp(
  options?: UseMutationOptions<RemoveGitHubAppResponse, ApiError, string>,
) {
  const invalidate = useInvalidateGitHubApps()
  return useMutation({
    ...options,
    mutationFn: (githubAppKey) => adminGithubConnectionApi.removeApp(githubAppKey),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

// Staff a registered App with its Bot user, so work can be attributed to it.
export function useCreateAdminGitHubBotUser(
  options?: UseMutationOptions<CreateUserResponse, ApiError, string>,
) {
  const invalidate = useInvalidateGitHubApps()
  return useMutation({
    ...options,
    mutationFn: (githubAppKey) => adminGithubConnectionApi.createBotUser(githubAppKey),
    onSuccess: async (...args) => {
      await invalidate()
      await options?.onSuccess?.(...args)
    },
  })
}

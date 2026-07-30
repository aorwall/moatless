import type {
  CreateUserResponse,
  GitHubAppRegistrationResponse,
  GitHubInstallationBindingRequest,
  GitHubInstallationBindingResponse,
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

export function useRegisterAdminGitHubApp(
  options?: UseMutationOptions<GitHubAppRegistrationResponse, ApiError, RegisterGitHubAppRequest>,
) {
  const invalidate = useInvalidateGitHubInstallation()
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
  const invalidate = useInvalidateGitHubInstallation()
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
  const invalidate = useInvalidateGitHubInstallation()
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
  const invalidate = useInvalidateGitHubInstallation()
  return useMutation({
    ...options,
    mutationFn: (githubAppKey) => adminGithubConnectionApi.createBotUser(githubAppKey),
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

import type {
  BranchListResponse,
  CreateRepositoryRequest,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/api-client'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { repositoriesApi } from '@/features/repositories/api/repositories'
import { queryKeys } from '@/lib/api/query-keys'

// Query hooks

/**
 * Hook to list ALL repositories (including templates)
 * This is the base query that fetches all repositories from the API
 */
export function useRepositoriesList(
  options?: Omit<UseQueryOptions<RepositoryResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositories.list(),
    queryFn: () => repositoriesApi.listRepositories(),
    ...options,
  })
}

export function useRepository(
  repoId: string,
  options?: Omit<UseQueryOptions<RepositoryResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositories.detail(repoId),
    queryFn: () => repositoriesApi.getRepository(repoId),
    enabled: !!repoId,
    ...options,
  })
}

// Mutation hooks
export function useCreateRepository(
  options?: UseMutationOptions<RepositoryResponse, Error, CreateRepositoryRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateRepositoryRequest) => repositoriesApi.createRepository(request),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      // Invalidate repositories list to refresh with new repository
      await queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() })
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useUpdateRepository(
  options?: UseMutationOptions<
    RepositoryResponse,
    Error,
    { repoId: string; request: UpdateRepositoryRequest }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ repoId, request }) => repositoriesApi.updateRepository(repoId, request),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: queryKeys.repositories.detail(variables.repoId),
        }),
        queryClient.invalidateQueries({ queryKey: queryKeys.repositories.list() }),
        queryClient.invalidateQueries({
          queryKey: queryKeys.repositories.branches(variables.repoId),
        }),
      ])
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useVerifyRepositoryAccess(
  options?: UseMutationOptions<RepositoryResponse, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.verifyAccess(repoId),
    ...options,
    onSuccess: async (data, repoId, onMutateResult, mutationContext) => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: queryKeys.repositories.detail(repoId),
        }),
        queryClient.invalidateQueries({ queryKey: queryKeys.repositories.list() }),
        queryClient.invalidateQueries({
          queryKey: queryKeys.repositories.branches(repoId),
        }),
      ])
      await options?.onSuccess?.(data, repoId, onMutateResult, mutationContext)
    },
  })
}

export function useDeleteRepository(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.deleteRepository(repoId),
    ...options,
    onSuccess: async (data, repoId, onMutateResult, mutationContext) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.repositories.detail(repoId),
      })
      // Invalidate repositories list
      await queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() })
      await options?.onSuccess?.(data, repoId, onMutateResult, mutationContext)
    },
  })
}

export function useConvertToTemplate(
  options?: UseMutationOptions<RepositoryResponse, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.convertToTemplate(repoId),
    ...options,
    onSuccess: async (data, repoId, onMutateResult, mutationContext) => {
      // Invalidate specific repository detail
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: queryKeys.repositories.detail(repoId),
        }),
        // Invalidate repositories list
        queryClient.invalidateQueries({ queryKey: queryKeys.repositories.all() }),
      ])
      await options?.onSuccess?.(data, repoId, onMutateResult, mutationContext)
    },
  })
}

export function useBranches(
  repoId: string,
  options?: Omit<UseQueryOptions<BranchListResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositories.branches(repoId),
    queryFn: () => repositoriesApi.getBranches(repoId),
    enabled: !!repoId,
    staleTime: 60_000, // Cache for 1 minute
    ...options,
  })
}

import type {
  BranchListResponse,
  CreateRepositoryRequest,
  FileSearchResponse,
  RepositoryResponse,
  UpdateRepositoryRequest,
  VerificationResponse,
} from '@moatless/types'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { repositoriesApi } from '@/features/repositories/api/repositories'
import { queryKeys } from '@/lib/api/query-keys'

// Query hooks
export function useRepositoriesList(
  options?: Omit<UseQueryOptions<Array<RepositoryResponse>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositoriesList(),
    queryFn: () => repositoriesApi.listRepositories(),
    ...options,
  })
}

export function useRepository(
  repoId: string,
  options?: Omit<UseQueryOptions<RepositoryResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositoryDetail(repoId),
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
    onSuccess: (data, variables, context) => {
      // Invalidate repositories list to refresh with new repository
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
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
    onSuccess: (data, variables, context) => {
      // Invalidate specific repository detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryDetail(variables.repoId),
      })
      // Invalidate repositories list
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useDeleteRepository(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.deleteRepository(repoId),
    onSuccess: (data, repoId, context) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.repositoryDetail(repoId),
      })
      // Invalidate repositories list
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, repoId, context)
    },
    ...options,
  })
}

export function useSearchFiles(
  repoId: string,
  query: string,
  options?: Omit<UseQueryOptions<FileSearchResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositoryFileSearch(repoId, query),
    queryFn: () => repositoriesApi.searchFiles(repoId, { query, limit: 10 }),
    enabled: !!repoId && query.length >= 3,
    staleTime: 30000, // Cache for 30 seconds
    ...options,
  })
}

export function useBranches(
  repoId: string,
  options?: Omit<UseQueryOptions<BranchListResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.repositoryBranches(repoId),
    queryFn: () => repositoriesApi.getBranches(repoId),
    enabled: !!repoId,
    staleTime: 60000, // Cache for 1 minute
    ...options,
  })
}

export function useVerifyRepository(
  options?: UseMutationOptions<VerificationResponse, Error, string>,
) {
  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.verifyRepository(repoId),
    ...options,
  })
}

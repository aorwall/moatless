import type {
  BranchListResponse,
  CreateRepositoryRequest,
  RepositoryResponse,
  TemplateRepository,
  UpdateRepositoryRequest,
} from '@moatless/types'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo } from 'react'
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
    queryKey: queryKeys.repositoriesList(),
    queryFn: () => repositoriesApi.listRepositories(),
    ...options,
  })
}

/**
 * Hook to list only template repositories (for vibe task creation)
 * Template repositories are source templates that can be cloned to create new local repositories
 * Filters client-side from the main repositories list for better caching
 */
export function useTemplateRepositories(options?: { enabled?: boolean }) {
  const { data: allRepos, ...queryResult } = useRepositoriesList({ enabled: options?.enabled })

  const templates = useMemo(() => {
    if (!allRepos) {
      return []
    }
    return allRepos.filter((repo): repo is TemplateRepository => repo.kind === 'template')
  }, [allRepos])

  return {
    ...queryResult,
    data: templates,
  }
}

/**
 * Hook to list non-template repositories (for regular task creation)
 * Excludes template repositories, showing only local and remote repositories
 * Filters client-side from the main repositories list for better caching
 */
export function useNonTemplateRepositories(options?: { enabled?: boolean }) {
  const { data: allRepos, ...queryResult } = useRepositoriesList({ enabled: options?.enabled })

  const nonTemplates = useMemo(() => {
    if (!allRepos) {
      return []
    }
    return allRepos.filter((repo) => repo.kind !== 'template')
  }, [allRepos])

  return {
    ...queryResult,
    data: nonTemplates,
  }
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
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate repositories list to refresh with new repository
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
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
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate specific repository detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryDetail(variables.repoId),
      })
      // Invalidate repositories list
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useDeleteRepository(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.deleteRepository(repoId),
    onSuccess: (data, repoId, onMutateResult, mutationContext) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.repositoryDetail(repoId),
      })
      // Invalidate repositories list
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, repoId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useConvertToTemplate(
  options?: UseMutationOptions<RepositoryResponse, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (repoId: string) => repositoriesApi.convertToTemplate(repoId),
    onSuccess: (data, repoId, onMutateResult, mutationContext) => {
      // Invalidate specific repository detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.repositoryDetail(repoId),
      })
      // Invalidate repositories list
      queryClient.invalidateQueries({ queryKey: queryKeys.repositories() })
      options?.onSuccess?.(data, repoId, onMutateResult, mutationContext)
    },
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

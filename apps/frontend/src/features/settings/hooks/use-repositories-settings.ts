import type {
  CreateRepositoryRequest,
  RepositoryConfig,
  UpdateRepositoryRequest,
} from '@moatless/types/types/repository'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { repositoriesApi } from '@/features/repositories/api/repositories'
import type { ApiError } from '@/lib/api/client'

export function useRepositoriesSettings() {
  const queryClient = useQueryClient()

  const {
    data: repositories = [],
    isLoading,
    error,
  } = useQuery({
    queryKey: ['repositories'],
    queryFn: repositoriesApi.listRepositories,
  })

  const createRepositoryMutation = useMutation<RepositoryConfig, ApiError, CreateRepositoryRequest>(
    {
      mutationFn: repositoriesApi.createRepository,
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: ['repositories'] })
      },
    },
  )

  const updateRepositoryMutation = useMutation<
    RepositoryConfig,
    ApiError,
    { id: string; data: UpdateRepositoryRequest }
  >({
    mutationFn: ({ id, data }) => repositoriesApi.updateRepository(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['repositories'] })
    },
  })

  const deleteRepositoryMutation = useMutation<void, ApiError, string>({
    mutationFn: repositoriesApi.deleteRepository,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['repositories'] })
    },
  })

  return {
    repositories,
    isLoading,
    error,
    createRepository: createRepositoryMutation.mutate,
    updateRepository: updateRepositoryMutation.mutate,
    deleteRepository: deleteRepositoryMutation.mutate,
    isCreating: createRepositoryMutation.isPending,
    isUpdating: updateRepositoryMutation.isPending,
    isDeleting: deleteRepositoryMutation.isPending,
  }
}

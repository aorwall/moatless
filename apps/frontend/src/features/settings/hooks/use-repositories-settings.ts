import type { UpdateRepositoryRequest } from '@moatless/shared/types/repository'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { repositoriesApi } from '@/features/repositories/api/repositories'

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

  const createRepositoryMutation = useMutation({
    mutationFn: repositoriesApi.createRepository,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['repositories'] })
    },
  })

  const updateRepositoryMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateRepositoryRequest }) =>
      repositoriesApi.updateRepository(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['repositories'] })
    },
  })

  const deleteRepositoryMutation = useMutation({
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

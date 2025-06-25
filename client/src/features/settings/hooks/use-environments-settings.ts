import type { UpdateEnvironmentRequest } from '@shared/types/environment'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { environmentsApi } from '@/features/environments/api/environments'

export function useEnvironmentsSettings() {
  const queryClient = useQueryClient()

  const {
    data: environments = [],
    isLoading,
    error,
  } = useQuery({
    queryKey: ['environments'],
    queryFn: environmentsApi.listEnvironments,
  })

  const createEnvironmentMutation = useMutation({
    mutationFn: environmentsApi.createEnvironment,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['environments'] })
    },
  })

  const updateEnvironmentMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateEnvironmentRequest }) =>
      environmentsApi.updateEnvironment(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['environments'] })
    },
  })

  const deleteEnvironmentMutation = useMutation({
    mutationFn: environmentsApi.deleteEnvironment,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['environments'] })
    },
  })

  return {
    environments,
    isLoading,
    error,
    createEnvironment: createEnvironmentMutation.mutate,
    updateEnvironment: updateEnvironmentMutation.mutate,
    deleteEnvironment: deleteEnvironmentMutation.mutate,
    isCreating: createEnvironmentMutation.isPending,
    isUpdating: updateEnvironmentMutation.isPending,
    isDeleting: deleteEnvironmentMutation.isPending,
  }
}

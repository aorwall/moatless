import type { CreatePluginRequest, PluginResponse } from '@moatless/api-client'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { pluginsApi } from '@/features/plugins/api/plugins'
import { queryKeys } from '@/lib/api/query-keys'

/** Every registered plugin. Visible to everyone; only admins may change the set. */
export function usePlugins(
  options?: Omit<UseQueryOptions<PluginResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.list(),
    queryFn: () => pluginsApi.listPlugins(),
    ...options,
  })
}

/** Plugins linked to one repository. */
export function useRepoPlugins(
  repoId: string,
  options?: Omit<UseQueryOptions<PluginResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.repoList(repoId),
    queryFn: () => pluginsApi.listRepoPlugins(repoId),
    enabled: !!repoId,
    ...options,
  })
}

export function useCreatePlugin(
  options?: UseMutationOptions<PluginResponse, Error, CreatePluginRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreatePluginRequest) => pluginsApi.createPlugin(request),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useDeletePlugin(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (pluginId: string) => pluginsApi.deletePlugin(pluginId),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

import type {
  ActivationResponse,
  ClearPluginActivationParams,
  CreatePluginRequest,
  EffectivePluginResponse,
  PluginResponse,
  PluginSkillResponse,
  SetActivationRequest,
} from '@moatless/api-client'
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

/**
 * The skills inside one plugin. Reading this syncs the plugin from its source
 * first, so opening the page both shows what is there now and leaves it stored.
 */
export function usePluginSkills(
  pluginId: string,
  options?: Omit<UseQueryOptions<PluginSkillResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.skills(pluginId),
    queryFn: () => pluginsApi.listPluginSkills(pluginId),
    enabled: !!pluginId,
    ...options,
  })
}

/** The activation records that apply to this user for one plugin. */
export function usePluginActivations(
  pluginId: string,
  options?: Omit<UseQueryOptions<ActivationResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.activations(pluginId),
    queryFn: () => pluginsApi.listPluginActivations(pluginId),
    enabled: !!pluginId,
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

/** What this user receives, after global defaults and their own records resolve. */
export function useEffectivePlugins(
  options?: Omit<UseQueryOptions<EffectivePluginResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.plugins.effective(),
    queryFn: () => pluginsApi.listEffectivePlugins(),
    ...options,
  })
}

export function useSetActivation(
  options?: UseMutationOptions<
    ActivationResponse,
    Error,
    { pluginId: string; request: SetActivationRequest }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ pluginId, request }: { pluginId: string; request: SetActivationRequest }) =>
      pluginsApi.setActivation(pluginId, request),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useClearActivation(
  options?: UseMutationOptions<
    void,
    Error,
    { pluginId: string; params: ClearPluginActivationParams }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ pluginId, params }: { pluginId: string; params: ClearPluginActivationParams }) =>
      pluginsApi.clearActivation(pluginId, params),
    ...options,
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.plugins.all() })
      await options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

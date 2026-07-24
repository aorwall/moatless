import type {
  AdapterConnectionResponse,
  CreateAdapterConnectionRequest,
  CreateLoopRequest,
  Loop,
  LoopRecentTaskItem,
  UpdateLoopRequest,
} from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { integrationsApi } from '@/features/integrations/api/integrations.api'
import { queryKeys } from '@/lib/api/query-keys'

export function useIntegrationAdapters() {
  return useQuery({
    queryKey: queryKeys.integrations.adapters(),
    queryFn: () => integrationsApi.listAdapters(),
    staleTime: 5 * 60_000,
  })
}

export function useIntegrationConnections(
  options?: Omit<UseQueryOptions<AdapterConnectionResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.integrations.connectionsList(),
    queryFn: () => integrationsApi.listAdapterConnections(),
    ...options,
  })
}

export function useCreateIntegrationConnection() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateAdapterConnectionRequest) =>
      integrationsApi.createAdapterConnection(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.connections() })
    },
  })
}

export function useTriggers(options?: Omit<UseQueryOptions<Loop[]>, 'queryKey' | 'queryFn'>) {
  return useQuery({
    queryKey: queryKeys.triggers.list(),
    queryFn: () => integrationsApi.listTriggers(),
    ...options,
  })
}

export function useCreateTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateLoopRequest) => integrationsApi.createTrigger(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.triggers.all() })
    },
  })
}

export function useUpdateTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateLoopRequest }) =>
      integrationsApi.updateTrigger(id, request),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.triggers.all() })
      queryClient.invalidateQueries({
        queryKey: queryKeys.triggers.detail(variables.id),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.triggers.recentTasks(variables.id),
      })
    },
  })
}

export function useDeleteTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (id: string) => integrationsApi.deleteTrigger(id),
    onSuccess: (_data, id) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.triggers.all() })
      queryClient.invalidateQueries({ queryKey: queryKeys.triggers.detail(id) })
      queryClient.invalidateQueries({
        queryKey: queryKeys.triggers.recentTasks(id),
      })
    },
  })
}

export function useTrigger(triggerId: string) {
  return useQuery({
    queryKey: queryKeys.triggers.detail(triggerId),
    queryFn: () => integrationsApi.getTrigger(triggerId),
    enabled: !!triggerId,
  })
}

export function useTriggerRecentTasks(triggerId: string, opts?: { limit?: number }) {
  return useQuery<LoopRecentTaskItem[]>({
    queryKey: queryKeys.triggers.recentTasks(triggerId),
    queryFn: () => integrationsApi.listTriggerRecentTasks(triggerId, { limit: opts?.limit ?? 10 }),
    enabled: !!triggerId,
  })
}

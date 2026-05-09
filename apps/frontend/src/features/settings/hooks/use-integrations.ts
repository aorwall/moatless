import type {
  CreateProviderConnectionRequest,
  CreateTaskRouteRequest,
  CreateTaskTriggerRequest,
  ProviderConnectionResponse,
  TaskRouteResponse,
  TaskTriggerResponse,
  UpdateTaskRouteRequest,
  UpdateTaskTriggerRequest,
} from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { integrationsApi } from '@/features/settings/api/integrations.api'
import { queryKeys } from '@/lib/api/query-keys'

export function useIntegrationProviders() {
  return useQuery({
    queryKey: queryKeys.integrations.providers(),
    queryFn: () => integrationsApi.listProviders(),
    staleTime: 5 * 60_000,
  })
}

export function useProviderConnections(
  options?: Omit<UseQueryOptions<ProviderConnectionResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.integrations.providerConnectionsList(),
    queryFn: () => integrationsApi.listProviderConnections(),
    ...options,
  })
}

export function useCreateProviderConnection() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateProviderConnectionRequest) =>
      integrationsApi.createProviderConnection(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.providerConnections() })
    },
  })
}

export function useTaskRoutes(
  options?: Omit<UseQueryOptions<TaskRouteResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.integrations.taskRoutesList(),
    queryFn: () => integrationsApi.listTaskRoutes(),
    ...options,
  })
}

export function useCreateTaskRoute() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateTaskRouteRequest) => integrationsApi.createTaskRoute(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskRoutes() })
    },
  })
}

export function useUpdateTaskRoute() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateTaskRouteRequest }) =>
      integrationsApi.updateTaskRoute(id, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskRoutes() })
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskTriggers() })
    },
  })
}

export function useTaskTriggers(
  options?: Omit<UseQueryOptions<TaskTriggerResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.integrations.taskTriggersList(),
    queryFn: () => integrationsApi.listTaskTriggers(),
    ...options,
  })
}

export function useCreateTaskTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateTaskTriggerRequest) => integrationsApi.createTaskTrigger(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskTriggers() })
    },
  })
}

export function useUpdateTaskTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateTaskTriggerRequest }) =>
      integrationsApi.updateTaskTrigger(id, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskTriggers() })
    },
  })
}

export function useDeleteTaskTrigger() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (id: string) => integrationsApi.deleteTaskTrigger(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.integrations.taskTriggers() })
    },
  })
}

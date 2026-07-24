import type {
  AdapterConnectionResponse,
  CreateAdapterConnectionRequest,
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

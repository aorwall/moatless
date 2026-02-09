import type { ServerStatusResponse } from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { serverApi } from '../api/server'

// Server management hooks
export function useServerStatus(
  taskId: string,
  options?: Omit<UseQueryOptions<ServerStatusResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.serverStatus(taskId),
    queryFn: () => serverApi.getServerStatus(taskId),
    enabled: !!taskId,
    ...options,
  })
}

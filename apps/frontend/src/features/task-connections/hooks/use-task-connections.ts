import { useQuery } from '@tanstack/react-query'
import { taskConnectionsApi } from '@/features/task-connections/api/task-connections.api'
import { queryKeys } from '@/lib/api/query-keys'

interface UseTaskConnectionsParams {
  limit?: number
  offset?: number
  /** When true, fill missing/stale connection statuses with a live fetch. */
  refresh?: boolean
}

export function useTaskConnections(params?: UseTaskConnectionsParams) {
  return useQuery({
    queryKey: queryKeys.admin.taskConnections(params),
    queryFn: () => taskConnectionsApi.list(params),
  })
}

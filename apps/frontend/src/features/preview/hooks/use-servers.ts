import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { type ServerEntry, serversApi } from '../api/servers'

/**
 * Hook to fetch available servers for a task.
 * Returns the list of servers configured in the execution target (e.g., preview, file-explorer).
 */
export function useServers(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.servers(taskId),
    queryFn: () => serversApi.getServers(taskId),
    enabled: !!taskId,
    select: (data) => data.servers,
    // Servers don't change often — refetch less aggressively
    staleTime: 30_000,
  })
}

export type { ServerEntry }

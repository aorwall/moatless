import type { GitSyncStatusInfo } from '@moatless/shared/types/api'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useGitSyncStatus(taskId: string, enabled = true) {
  return useQuery<GitSyncStatusInfo>({
    queryKey: queryKeys.tasks.gitSyncStatus(taskId),
    queryFn: async () => {
      const response = await apiClient.get<GitSyncStatusInfo>(`/api/tasks/${taskId}/git-sync-status`)
      return response
    },
    enabled: !!taskId && enabled,
    staleTime: 30000, // Consider data stale after 30 seconds
    refetchInterval: 60000, // Refetch every minute
  })
}
import type { FileTreeWithChangesResponse } from '@moatless/types'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

interface FileTreeWithChangesParams {
  taskId: string
  path?: string
}

export function useFileTreeWithChanges({ taskId, path = '.' }: FileTreeWithChangesParams) {
  return useQuery<FileTreeWithChangesResponse>({
    queryKey: [...queryKeys.tasks.detail(taskId), 'file-tree-with-changes', { path }] as const,
    queryFn: async () => {
      const params = new URLSearchParams()
      if (path && path !== '.') params.append('path', path)

      const queryString = params.toString()
      const url = `/api/v1/tasks/${taskId}/file-tree-with-changes${queryString ? `?${queryString}` : ''}`

      const response = await apiClient.get<FileTreeWithChangesResponse>(url)
      return response
    },
    enabled: !!taskId,
    // Refresh every 5 seconds to catch new changes
    refetchInterval: 5000,
  })
}

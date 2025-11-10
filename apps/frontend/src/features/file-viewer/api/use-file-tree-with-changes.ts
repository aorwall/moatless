import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { getFileTreeWithChangesRoute } from '@moatless/api-contract/workspace.ts'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

type FileTreeWithChangesResponse = RouteSuccess<typeof getFileTreeWithChangesRoute>

interface FileTreeWithChangesParams {
  taskId: string
  path?: string
}

export function useFileTreeWithChanges({ taskId, path = '.' }: FileTreeWithChangesParams) {
  return useQuery<FileTreeWithChangesResponse>({
    queryKey: queryKeys.tasks.fileTreeWithChanges(taskId, path),
    queryFn: async () => {
      const params = new URLSearchParams()
      if (path && path !== '.') {
        params.append('path', path)
      }

      const queryString = params.toString()
      const basePath = buildPath(getFileTreeWithChangesRoute, { taskId })
      const url = `${basePath}${queryString ? `?${queryString}` : ''}`

      return await apiClient.get<FileTreeWithChangesResponse>(url)
    },
    enabled: !!taskId,
  })
}

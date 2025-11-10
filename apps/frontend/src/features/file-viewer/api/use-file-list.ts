import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { listWorkspaceFilesRoute } from '@moatless/api-contract/workspace.ts'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

type FileListResponse = RouteSuccess<typeof listWorkspaceFilesRoute>

interface FileListParams {
  taskId: string
  path?: string
  recursive?: boolean
}

export function useFileList({ taskId, path = '/', recursive = false }: FileListParams) {
  return useQuery<FileListResponse>({
    queryKey: queryKeys.tasks.fileList(taskId, path, recursive),
    queryFn: async () => {
      const params = new URLSearchParams()
      if (path) {
        params.append('path', path)
      }
      if (recursive) {
        params.append('recursive', 'true')
      }

      const queryString = params.toString()
      const basePath = buildPath(listWorkspaceFilesRoute, { taskId })
      const url = `${basePath}${queryString ? `?${queryString}` : ''}`

      return await apiClient.get<FileListResponse>(url)
    },
    enabled: !!taskId,
  })
}

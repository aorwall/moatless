import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

interface FileListParams {
  taskId: string
  path?: string
  recursive?: boolean
}

interface FileListResponse {
  files: string[]
  totalCount: number
}

export function useFileList({ taskId, path = '/', recursive = false }: FileListParams) {
  return useQuery<FileListResponse>({
    queryKey: queryKeys.taskFileList(taskId, path, recursive),
    queryFn: async () => {
      const params = new URLSearchParams()
      if (path && path !== '/') params.append('path', path)
      if (recursive) params.append('recursive', 'true')

      const queryString = params.toString()
      const url = `/tasks/${taskId}/files${queryString ? `?${queryString}` : ''}`

      const response = await apiClient.get<FileListResponse>(url)
      return response.data
    },
    enabled: !!taskId,
  })
}

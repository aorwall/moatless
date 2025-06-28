import { useMutation, useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryClient } from '@/lib/api/query-client'
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

interface FileContentResponse {
  content: string
  encoding: 'utf-8'
}

interface FileSearchResponse {
  results: Array<{
    path: string
    matches: number
  }>
}

export function useFileViewer(taskId: string) {
  const useFileList = ({ path = '/', recursive = false }: Omit<FileListParams, 'taskId'> = {}) => {
    return useQuery<FileListResponse>({
      queryKey: queryKeys.taskFileList(taskId, path, recursive),
      queryFn: async () => {
        const params = new URLSearchParams()
        if (path) params.append('path', path)
        if (recursive) params.append('recursive', 'true')

        const queryString = params.toString()
        const url = `/api/v1/tasks/${taskId}/files${queryString ? `?${queryString}` : ''}`

        const response = await apiClient.get<FileListResponse>(url)
        return response
      },
      enabled: !!taskId,
    })
  }

  const useFileContent = (filePath: string) => {
    return useQuery<FileContentResponse>({
      queryKey: queryKeys.taskFileContent(taskId, filePath),
      queryFn: async () => {
        const encodedPath = encodeURIComponent(filePath)
        const response = await apiClient.get<FileContentResponse>(
          `/api/v1/tasks/${taskId}/files/${encodedPath}`,
        )
        return response
      },
      enabled: !!taskId && !!filePath,
    })
  }

  const useFileSearch = () => {
    return useMutation<FileSearchResponse, Error, string>({
      mutationFn: async (query: string) => {
        const response = await apiClient.post<FileSearchResponse>(
          `/api/v1/tasks/${taskId}/search-files`,
          { query },
        )
        return response
      },
      onSuccess: (data, query) => {
        queryClient.setQueryData(queryKeys.taskFileSearch(taskId, query), data)
      },
    })
  }

  return {
    useFileList,
    useFileContent,
    useFileSearch,
  }
}

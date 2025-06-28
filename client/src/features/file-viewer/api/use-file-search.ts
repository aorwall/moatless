import { useMutation } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryClient } from '@/lib/api/query-client'
import { queryKeys } from '@/lib/api/query-keys'

interface FileSearchResponse {
  results: Array<{
    path: string
    matches: number
  }>
}

export function useFileSearch(taskId: string) {
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
import type { FileSearchResponse } from '@moatless/types'
import { useMutation } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'

export function useFileSearch(taskId: string) {
  return useMutation<FileSearchResponse, Error, string>({
    mutationFn: async (query: string) => {
      const response = await apiClient.post<FileSearchResponse>(
        `/api/v1/tasks/${taskId}/search-files`,
        { query },
      )
      return response
    },
  })
}

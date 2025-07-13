import type { FileContentResponse } from '@moatless/types'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useFileContent(taskId: string, filePath: string) {
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
    // Reduce cache time for file content to detect changes faster
    staleTime: 30 * 1000, // 30 seconds instead of 5 minutes
    gcTime: 60 * 1000, // 1 minute instead of 10 minutes
  })
}

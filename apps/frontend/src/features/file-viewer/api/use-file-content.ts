import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { readWorkspaceFileRoute } from '@moatless/api-contract/workspace.ts'
import { useQuery } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

type FileContentResponse = RouteSuccess<typeof readWorkspaceFileRoute>

export function useFileContent(taskId: string, filePath: string) {
  return useQuery<FileContentResponse>({
    queryKey: queryKeys.tasks.fileContent(taskId, filePath),
    queryFn: async () => {
      const path = buildPath(readWorkspaceFileRoute, { taskId, filePath })
      const response = await apiClient.get<FileContentResponse>(path)
      return response
    },
    enabled: !!taskId && !!filePath,
    // Reduce cache time for file content to detect changes faster
    staleTime: 30 * 1000, // 30 seconds instead of 5 minutes
    gcTime: 60 * 1000, // 1 minute instead of 10 minutes
  })
}

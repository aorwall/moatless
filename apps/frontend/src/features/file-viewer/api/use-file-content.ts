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
      return await apiClient.get<FileContentResponse>(path)
    },
    enabled: !!taskId && !!filePath,
    staleTime: 30 * 1000,
    gcTime: 60 * 1000,
  })
}

import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { searchWorkspaceFilesRoute } from '@moatless/api-contract/workspace.ts'
import { useMutation } from '@tanstack/react-query'
import { apiClient } from '@/lib/api/client'

type FileSearchResponse = RouteSuccess<typeof searchWorkspaceFilesRoute>

export function useFileSearch(taskId: string) {
  return useMutation<FileSearchResponse, Error, string>({
    mutationFn: async (query: string) => {
      const path = buildPath(searchWorkspaceFilesRoute, { taskId })
      const response = await apiClient.post<FileSearchResponse>(path, {
        query,
      })
      return response
    },
  })
}

import type { FileSearchResult } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { searchFilesHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import { useMutation } from '@tanstack/react-query'

interface WorkspaceFileSearchResponse {
  files: { path: string; matches: number }[]
  totalMatches: number
}

export function useFileSearch(taskId: string) {
  return useMutation<WorkspaceFileSearchResponse, Error, string>({
    mutationFn: async (query: string) => {
      const results = await unwrapResponse<FileSearchResult[]>(
        searchFilesHandler(taskId, { query }),
      )
      return {
        files: results.map((r) => ({ path: r.path, matches: 1 })),
        totalMatches: results.length,
      }
    },
  })
}

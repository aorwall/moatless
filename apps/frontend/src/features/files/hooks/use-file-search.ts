import { useMutation } from '@tanstack/react-query'
import { searchFilesByName } from '../api/files-api'
import type { SearchByNameResponse } from '../types'

export function useFileSearch(taskId: string) {
  return useMutation<
    SearchByNameResponse,
    Error,
    { query: string; caseSensitive?: boolean; maxResults?: number }
  >({
    mutationFn: (params) => searchFilesByName(taskId, params.query, params),
  })
}

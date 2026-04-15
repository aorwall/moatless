import { useMutation } from '@tanstack/react-query'
import { searchFilesByName } from '@/features/files/api/files-api'
import type { SearchByNameResponse } from '@/features/files/types'

export function useFileSearch(taskId: string) {
  return useMutation<SearchByNameResponse, Error, string>({
    mutationFn: (query: string) => searchFilesByName(taskId, query, { maxResults: 20 }),
  })
}

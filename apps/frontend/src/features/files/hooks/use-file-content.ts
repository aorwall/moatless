import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchFileContent } from '../api/files-api'
import type { FileContentResponse } from '../types'

export function useFileContent(taskId: string, filePath: string | undefined, withChanges = true) {
  return useQuery<FileContentResponse>({
    queryKey: [...queryKeys.tasks.files(taskId), 'content', filePath, withChanges],
    queryFn: () => fetchFileContent(taskId, filePath!, withChanges),
    enabled: !!filePath,
  })
}

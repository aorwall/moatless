import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'
import { fetchFileContent } from '../api/files-api'
import type { FileContentResponse } from '../types'

export function useFileContent(taskId: string, filePath: string | undefined, withChanges = true) {
  const { data: sandboxStatus } = useTaskSandboxStatusQuery(taskId)
  const isSandboxReady = sandboxStatus?.sandboxStatus === 'ready'

  return useQuery<FileContentResponse>({
    queryKey: queryKeys.tasks.fileContent(taskId, filePath, withChanges),
    queryFn: () =>
      filePath
        ? fetchFileContent(taskId, filePath, withChanges)
        : Promise.reject(new Error('filePath is required')),
    enabled: !!filePath && isSandboxReady,
  })
}

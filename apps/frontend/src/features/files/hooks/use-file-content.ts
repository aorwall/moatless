import { useQuery } from '@tanstack/react-query'
import { useTaskSandboxStatusQuery } from '@/features/tasks/hooks/use-task-queries'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchFileContent } from '../api/files-api'
import type { FileContentResponse } from '../types'

export function useFileContent(taskId: string, filePath: string | undefined, withChanges = true) {
  const { data: sandboxStatus } = useTaskSandboxStatusQuery(taskId)
  const isSandboxReady = sandboxStatus?.environmentStatus === 'ready'

  return useQuery<FileContentResponse>({
    queryKey: [...queryKeys.tasks.files(taskId), 'content', filePath, withChanges],
    queryFn: () =>
      filePath
        ? fetchFileContent(taskId, filePath, withChanges)
        : Promise.reject(new Error('filePath is required')),
    enabled: !!filePath && isSandboxReady,
  })
}

import { useQueries } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'
import { fetchFileTree } from '../api/files-api'
import type { FileTreeResponse } from '../types'

interface UseFileTreeResult {
  data: FileTreeResponse | undefined
  isLoading: boolean
  isHydrated: boolean
  error: Error | null
}

export function useFileTree(taskId: string): UseFileTreeResult {
  const { data: sandboxStatus } = useTaskSandboxStatusQuery(taskId)
  const isSandboxReady = sandboxStatus?.environmentStatus === 'ready'

  const [shallow, recursive] = useQueries({
    queries: [
      {
        queryKey: queryKeys.tasks.fileTree(taskId, 'shallow'),
        queryFn: () => fetchFileTree(taskId),
        refetchInterval: 30_000,
        enabled: isSandboxReady,
      },
      {
        queryKey: queryKeys.tasks.fileTree(taskId, 'recursive'),
        queryFn: () => fetchFileTree(taskId, undefined, true),
        refetchInterval: 30_000,
        enabled: isSandboxReady,
      },
    ],
  })

  const data = (recursive.data ?? shallow.data) as FileTreeResponse | undefined

  return {
    data,
    isLoading: shallow.isLoading && recursive.isLoading,
    isHydrated: recursive.isSuccess,
    error: (shallow.error ?? recursive.error) as Error | null,
  }
}

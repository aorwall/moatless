import { useQueries } from '@tanstack/react-query'
import { TASK_DATA_GC_TIME } from '@/lib/api/query-client'
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
  const isSandboxReady = sandboxStatus?.sandboxStatus === 'ready'

  // No polling: re-fetching a whole repo tree on a timer re-parsed and
  // re-garbaged the entire structure every 30s for as long as this panel was
  // open, which was a steady feed into the GC → cycle-collection pauses. Tree
  // changes arrive through the SSE tool-invalidation registry instead, which
  // invalidates `queryKeys.tasks.files(taskId)` after file-touching tools.
  const [shallow, recursive] = useQueries({
    queries: [
      {
        queryKey: queryKeys.tasks.fileTree(taskId, 'shallow'),
        queryFn: () => fetchFileTree(taskId),
        gcTime: TASK_DATA_GC_TIME,
        enabled: isSandboxReady,
      },
      {
        queryKey: queryKeys.tasks.fileTree(taskId, 'recursive'),
        queryFn: () => fetchFileTree(taskId, undefined, true),
        gcTime: TASK_DATA_GC_TIME,
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

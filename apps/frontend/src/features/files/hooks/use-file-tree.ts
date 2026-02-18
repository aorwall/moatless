import { useQuery } from '@tanstack/react-query'
import { useTaskWorkspaceStatusQuery } from '@/features/tasks/hooks/use-task-queries'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchFileTree } from '../api/files-api'
import type { FileTreeResponse } from '../types'

export function useFileTree(taskId: string, rootPath = '.') {
  const { data: workspaceStatus } = useTaskWorkspaceStatusQuery(taskId)
  const isSandboxReady = workspaceStatus?.environmentStatus === 'ready'

  return useQuery<FileTreeResponse>({
    queryKey: [...queryKeys.tasks.files(taskId), 'tree', rootPath],
    queryFn: () => fetchFileTree(taskId, rootPath !== '.' ? rootPath : undefined),
    refetchInterval: 30_000,
    enabled: isSandboxReady,
  })
}

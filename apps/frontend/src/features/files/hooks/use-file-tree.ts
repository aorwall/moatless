import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { fetchFileTree } from '../api/files-api'
import type { FileTreeResponse } from '../types'

export function useFileTree(taskId: string, rootPath = '.') {
  return useQuery<FileTreeResponse>({
    queryKey: [...queryKeys.tasks.files(taskId), 'tree', rootPath],
    queryFn: () => fetchFileTree(taskId, rootPath !== '.' ? rootPath : undefined),
    refetchInterval: 30_000,
  })
}

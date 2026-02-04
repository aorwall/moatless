import { useQuery } from '@tanstack/react-query'
import type { FileTreeNode, GitSummary } from './types'

export type { FileTreeNode, GitSummary }

interface FileTreeWithChangesResponse {
  tree: FileTreeNode[]
  gitSummary: GitSummary
}

export function useFileTree(rootPath = '.') {
  return useQuery<FileTreeWithChangesResponse>({
    queryKey: ['file-tree', rootPath],
    queryFn: async () => {
      const params = new URLSearchParams()
      if (rootPath !== '.') {
        params.set('path', rootPath)
      }
      const res = await fetch(`/api/tree?${params}`)
      if (!res.ok) {
        throw new Error(`Failed to fetch file tree: ${res.statusText}`)
      }
      return res.json()
    },
    refetchInterval: 10_000,
  })
}

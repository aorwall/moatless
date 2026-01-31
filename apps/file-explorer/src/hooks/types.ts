export interface FileTreeNode {
  name: string
  path: string
  type: 'file' | 'directory'
  isChanged: boolean
  gitStatus?: 'added' | 'modified' | 'deleted' | 'renamed'
  additions?: number
  deletions?: number
  isStaged?: boolean
  children?: FileTreeNode[]
}

export interface GitSummary {
  totalFiles: number
  totalAdditions: number
  totalDeletions: number
  totalStaged: number
  totalModified: number
  compareBase: string
  hasChanges: boolean
}

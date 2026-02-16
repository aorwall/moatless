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

export interface FileTreeResponse {
  tree: FileTreeNode[]
  gitSummary: GitSummary
}

export interface FileContentResponse {
  content: string
  oldContent?: string
  additions?: number
  deletions?: number
  isModified?: boolean
}

export interface SearchByNameResponse {
  files: Array<{ path: string; matches: number }>
  totalMatches: number
}

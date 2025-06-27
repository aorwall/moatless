export interface FileTreeNode {
  name: string
  path: string
  type: 'file' | 'directory'
  children?: FileTreeNode[]
}

export interface FileTreeState {
  expandedPaths: Set<string>
  selectedPath: string | null
  searchQuery: string
}

export interface FileViewerState {
  currentFile: string | null
  fileContent: string | null
  isLoading: boolean
  error: Error | null
}

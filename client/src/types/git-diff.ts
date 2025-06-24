// Git diff types (compatible with our needs, not gitdiff-parser directly)
export interface Hunk {
  content: string
  oldStart: number
  newStart: number
  oldLines: number
  newLines: number
}

export interface GitDiffFile {
  hunks: Array<Hunk>
  oldEndingNewLine: boolean
  newEndingNewLine: boolean
  oldMode: string
  newMode: string
  similarity?: number
  oldRevision: string
  newRevision: string
  oldPath: string
  newPath: string
  isBinary?: boolean
  type: 'add' | 'delete' | 'modify' | 'rename' | 'copy'
}

export interface GitDiffResponse {
  files: Array<GitDiffFile>
  total_files: number
  total_additions: number
  total_deletions: number
  commit_hash?: string
  branch?: string
  diff?: string
}

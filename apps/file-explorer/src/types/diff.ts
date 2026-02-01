/**
 * Represents a word-level change within a line
 */
export interface WordChange {
  type: 'equal' | 'add' | 'del'
  value: string
}

/**
 * Represents a single line in a diff
 */
export interface DiffLineData {
  type: 'normal' | 'add' | 'del' | 'hunk-header'
  content: string
  oldNumber?: number | undefined
  newNumber?: number | undefined
  /** Word-level changes for highlighting partial modifications */
  changes?: WordChange[] | undefined
}

/**
 * Represents a collapsed section of unchanged lines
 */
export interface HiddenChunkData {
  type: 'hidden-chunk'
  /** Number of lines that are hidden */
  lineCount: number
  /** The actual lines that are hidden (for expansion) */
  lines: DiffLineData[]
  /** Start line number in old file */
  oldStart: number
  /** Start line number in new file */
  newStart: number
}

/**
 * Union type for items that can appear in diff lines array
 */
export type DiffLineItem = DiffLineData | HiddenChunkData

/**
 * Helper to check if an item is a hidden chunk
 */
export function isHiddenChunk(item: DiffLineItem): item is HiddenChunkData {
  return item.type === 'hidden-chunk'
}

/**
 * Represents a hunk (section) of changes in a diff
 */
export interface DiffHunk {
  oldStart: number
  oldLines: number
  newStart: number
  newLines: number
  header: string
  lines: DiffLineItem[]
}

/**
 * File change status
 */
export type FileStatus = 'added' | 'modified' | 'deleted' | 'renamed'

/**
 * Represents a file in a diff
 */
export interface DiffFile {
  filename: string
  /** For renames, the original filename */
  oldFilename?: string | undefined
  status: FileStatus
  additions: number
  deletions: number
  hunks: DiffHunk[]
  /** Programming language for syntax highlighting */
  language?: string | undefined
}

/**
 * Result of parsing a unified diff
 */
export interface ParsedDiff {
  files: DiffFile[]
}

/**
 * Props for the diff lines component
 */
export interface DiffLinesProps {
  lines: DiffLineItem[]
  /** Programming language for syntax highlighting */
  language?: string | undefined
  /** Map of chunk index to expansion state */
  chunkExpansion?: Map<number, ChunkExpansionState> | undefined
  /** Callback to expand N lines from top of a chunk */
  onExpandTop?: ((chunkIndex: number, count: number) => void) | undefined
  /** Callback to expand N lines from bottom of a chunk */
  onExpandBottom?: ((chunkIndex: number, count: number) => void) | undefined
  /** Callback to expand all lines in a chunk */
  onExpandAll?: ((chunkIndex: number, totalLines: number) => void) | undefined
}

/**
 * Props for a single diff line
 */
export interface DiffLineProps {
  line: DiffLineData
  /** Programming language for syntax highlighting */
  language?: string | undefined
}

/**
 * State for partial expansion of a hidden chunk
 */
export interface ChunkExpansionState {
  /** Number of lines revealed from top */
  revealedFromTop: number
  /** Number of lines revealed from bottom */
  revealedFromBottom: number
}

/**
 * Position of hidden chunk within the diff
 */
export type HiddenChunkPosition = 'start' | 'middle' | 'end'

/**
 * Props for hidden chunk placeholder
 */
export interface HiddenChunkProps {
  /** Total number of lines in this chunk */
  lineCount: number
  /** Position of chunk: start (show expand down), end (show expand up), middle (show expand all) */
  position?: HiddenChunkPosition | undefined
  /** Number of lines already revealed from top */
  revealedFromTop?: number | undefined
  /** Number of lines already revealed from bottom */
  revealedFromBottom?: number | undefined
  /** Callback to expand N lines from top */
  onExpandTop?: ((count: number) => void) | undefined
  /** Callback to expand N lines from bottom */
  onExpandBottom?: ((count: number) => void) | undefined
  /** Callback to expand all lines */
  onExpandAll?: (() => void) | undefined
}

/**
 * Props for the diff gutter (line numbers + change indicator)
 */
export interface DiffGutterProps {
  oldNumber?: number | undefined
  newNumber?: number | undefined
  type: DiffLineData['type']
}

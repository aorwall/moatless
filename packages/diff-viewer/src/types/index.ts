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
 * Props for the main DiffViewer component
 */
export interface DiffViewerProps {
  /** The unified diff string to parse and display */
  diff?: string | undefined
  /** Pre-parsed diff files (alternative to diff string) */
  files?: DiffFile[] | undefined
  /** View mode: unified (default) or split */
  viewMode?: 'unified' | 'split' | undefined
  /** Whether to show the sidebar with file list */
  showSidebar?: boolean | undefined
  /** Default width of the sidebar in pixels */
  sidebarWidth?: number | undefined
  /** Callback when a file is selected in the sidebar */
  onFileSelect?: ((filename: string) => void) | undefined
  /** Currently selected file */
  selectedFile?: string | undefined
  /** Whether files are collapsed by default */
  defaultCollapsed?: boolean | undefined
  /** Class name for the container */
  className?: string | undefined
}

/**
 * Props for individual file diff card
 */
export interface DiffFileCardProps {
  file: DiffFile
  /** Whether the file card is collapsed */
  collapsed?: boolean | undefined
  /** Callback when collapse state changes */
  onCollapseChange?: ((collapsed: boolean) => void) | undefined
  /** Set of expanded hidden chunk indices */
  expandedChunks?: Set<number> | undefined
  /** Callback when a hidden chunk is expanded/collapsed */
  onChunkToggle?: ((chunkIndex: number) => void) | undefined
}

/**
 * Props for the diff lines component
 */
export interface DiffLinesProps {
  lines: DiffLineItem[]
  /** View mode */
  viewMode?: 'unified' | 'split' | undefined
  /** Starting line number for old content */
  oldStartLine?: number | undefined
  /** Starting line number for new content */
  newStartLine?: number | undefined
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
  /** View mode */
  viewMode?: 'unified' | 'split' | undefined
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
  /** Number of lines already revealed from top (only used for position='start') */
  revealedFromTop?: number | undefined
  /** Number of lines already revealed from bottom (only used for position='end') */
  revealedFromBottom?: number | undefined
  /** Callback to expand N lines from top (for position='start') */
  onExpandTop?: ((count: number) => void) | undefined
  /** Callback to expand N lines from bottom (for position='end') */
  onExpandBottom?: ((count: number) => void) | undefined
  /** Callback to expand all lines (for position='middle') */
  onExpandAll?: (() => void) | undefined
}

/**
 * Props for file change stats (+X / -Y)
 */
export interface FileChangeStatsProps {
  additions: number
  deletions: number
  /** Whether to show zeros */
  showZeros?: boolean | undefined
  /** Size variant */
  size?: 'sm' | 'md' | undefined
}

/**
 * Props for the diff gutter (line numbers + change indicator)
 */
export interface DiffGutterProps {
  oldNumber?: number | undefined
  newNumber?: number | undefined
  type: DiffLineData['type']
}

/**
 * Props for the file header in a diff card
 */
export interface DiffFileHeaderProps {
  file: DiffFile
  collapsed?: boolean | undefined
  onCollapseChange?: ((collapsed: boolean) => void) | undefined
}

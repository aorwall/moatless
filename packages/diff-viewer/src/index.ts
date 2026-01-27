// Types

// Components
export {
  DiffFileCard,
  DiffFileHeader,
  DiffGutter,
  DiffLine,
  DiffLines,
  FileChangeStats,
  HiddenChunk,
  SyntaxToken,
} from './components/index.js'
export type {
  ChunkExpansionState,
  DiffFile,
  DiffFileCardProps,
  DiffFileHeaderProps,
  DiffGutterProps,
  DiffHunk,
  DiffLineData,
  DiffLineItem,
  DiffLineProps,
  DiffLinesProps,
  DiffViewerProps,
  FileChangeStatsProps,
  FileStatus,
  HiddenChunkData,
  HiddenChunkPosition,
  HiddenChunkProps,
  ParsedDiff,
  WordChange,
} from './types/index.js'
export { isHiddenChunk } from './types/index.js'
// Utils
export {
  computeCharDiff,
  computeWordDiff,
  getFileStatus,
  type HighlightToken,
  hasWordChanges,
  highlightLine,
  isLanguageSupported,
  type ParseDiffOptions,
  parseDiff,
  tokenize,
} from './utils/index.js'

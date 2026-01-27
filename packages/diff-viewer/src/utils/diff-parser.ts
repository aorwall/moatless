import type {
  DiffFile,
  DiffHunk,
  DiffLineData,
  DiffLineItem,
  FileStatus,
  HiddenChunkData,
  ParsedDiff,
} from '../types/index.js'
import { computeWordDiff } from './word-diff.js'

/**
 * Options for parsing a unified diff
 */
export interface ParseDiffOptions {
  /** Collapse consecutive unchanged lines into hidden chunks (default: true) */
  collapseUnchanged?: boolean
  /** Minimum consecutive unchanged lines to collapse (default: 8) */
  collapseThreshold?: number
  /** Lines to show before/after collapsed section (default: 3) */
  contextLines?: number
}

const DEFAULT_OPTIONS: Required<ParseDiffOptions> = {
  collapseUnchanged: true,
  collapseThreshold: 8,
  contextLines: 3,
}

/**
 * Parse a unified diff string into structured DiffFile objects
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Diff parsing inherently requires handling many edge cases
export function parseDiff(diffString: string, options?: ParseDiffOptions): ParsedDiff {
  if (!diffString || diffString.trim() === '') {
    return { files: [] }
  }

  const files: DiffFile[] = []
  const lines = diffString.split('\n')

  let currentFile: DiffFile | null = null
  let currentHunk: DiffHunk | null = null
  let i = 0

  while (i < lines.length) {
    const line = lines[i] ?? ''

    // Check for diff header (diff --git a/file b/file)
    if (line.startsWith('diff --git')) {
      // Save previous file if exists
      if (currentFile) {
        if (currentHunk) {
          currentFile.hunks.push(currentHunk)
        }
        files.push(currentFile)
      }

      // Parse filename from diff --git line
      const match = line.match(/^diff --git a\/(.+) b\/(.+)$/)
      const oldFilename = match?.[1] ?? ''
      const newFilename = match?.[2] ?? oldFilename

      currentFile = {
        filename: newFilename,
        oldFilename: oldFilename !== newFilename ? oldFilename : undefined,
        status: 'modified',
        additions: 0,
        deletions: 0,
        hunks: [],
      }
      currentHunk = null
      i++
      continue
    }

    // Check for new file mode
    if (line.startsWith('new file mode') && currentFile) {
      currentFile.status = 'added'
      i++
      continue
    }

    // Check for deleted file mode
    if (line.startsWith('deleted file mode') && currentFile) {
      currentFile.status = 'deleted'
      i++
      continue
    }

    // Check for rename
    if (line.startsWith('rename from') && currentFile) {
      const match = line.match(/^rename from (.+)$/)
      if (match?.[1]) {
        currentFile.oldFilename = match[1]
        currentFile.status = 'renamed'
      }
      i++
      continue
    }

    if (line.startsWith('rename to') && currentFile) {
      const match = line.match(/^rename to (.+)$/)
      if (match?.[1]) {
        currentFile.filename = match[1]
      }
      i++
      continue
    }

    // Check for hunk header (@@ -x,y +a,b @@)
    const hunkMatch = line.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/)
    if (hunkMatch && currentFile) {
      // Save previous hunk
      if (currentHunk) {
        currentFile.hunks.push(currentHunk)
      }

      currentHunk = {
        oldStart: Number.parseInt(hunkMatch[1] ?? '0', 10),
        oldLines: Number.parseInt(hunkMatch[2] ?? '1', 10),
        newStart: Number.parseInt(hunkMatch[3] ?? '0', 10),
        newLines: Number.parseInt(hunkMatch[4] ?? '1', 10),
        header: line,
        lines: [
          {
            type: 'hunk-header',
            content: hunkMatch[5] ?? '',
          },
        ],
      }
      i++
      continue
    }

    // Parse diff content lines
    if (currentHunk && currentFile) {
      if (line.startsWith('+') && !line.startsWith('+++')) {
        currentHunk.lines.push({
          type: 'add',
          content: line.slice(1),
          newNumber: calculateNewLineNumber(currentHunk),
        })
        currentFile.additions++
        i++
        continue
      }

      if (line.startsWith('-') && !line.startsWith('---')) {
        currentHunk.lines.push({
          type: 'del',
          content: line.slice(1),
          oldNumber: calculateOldLineNumber(currentHunk),
        })
        currentFile.deletions++
        i++
        continue
      }

      if (line.startsWith(' ') || line === '') {
        const content = line.startsWith(' ') ? line.slice(1) : line
        currentHunk.lines.push({
          type: 'normal',
          content,
          oldNumber: calculateOldLineNumber(currentHunk),
          newNumber: calculateNewLineNumber(currentHunk),
        })
        i++
        continue
      }
    }

    // Skip other lines (---, +++, index, etc.)
    i++
  }

  // Don't forget the last file and hunk
  if (currentFile) {
    if (currentHunk) {
      currentFile.hunks.push(currentHunk)
    }
    files.push(currentFile)
  }

  // Compute word-level diffs for modified lines
  // At this point, lines are all DiffLineData (before collapsing)
  for (const file of files) {
    for (const hunk of file.hunks) {
      addWordDiffs(hunk.lines as DiffLineData[])
    }
  }

  // Detect language from filename
  for (const file of files) {
    const lang = detectLanguage(file.filename)
    if (lang !== undefined) {
      file.language = lang
    }
  }

  // Collapse unchanged lines into hidden chunks if enabled
  // This converts some DiffLineData to HiddenChunkData
  const opts = { ...DEFAULT_OPTIONS, ...options }
  if (opts.collapseUnchanged) {
    for (const file of files) {
      for (const hunk of file.hunks) {
        hunk.lines = collapseUnchangedLines(
          hunk.lines as DiffLineData[],
          opts.collapseThreshold,
          opts.contextLines,
        )
      }
    }
  }

  return { files }
}

/**
 * Calculate the old line number for the current position in the hunk
 */
function calculateOldLineNumber(hunk: DiffHunk): number {
  let lineNum = hunk.oldStart
  for (const line of hunk.lines) {
    if ((line.type === 'normal' || line.type === 'del') && line.oldNumber !== undefined) {
      lineNum = line.oldNumber + 1
    }
  }
  return lineNum
}

/**
 * Calculate the new line number for the current position in the hunk
 */
function calculateNewLineNumber(hunk: DiffHunk): number {
  let lineNum = hunk.newStart
  for (const line of hunk.lines) {
    if ((line.type === 'normal' || line.type === 'add') && line.newNumber !== undefined) {
      lineNum = line.newNumber + 1
    }
  }
  return lineNum
}

/**
 * Add word-level diffs to consecutive add/del line pairs
 */
function addWordDiffs(lines: DiffLineData[]): void {
  let i = 0
  while (i < lines.length) {
    // Look for consecutive del lines followed by consecutive add lines
    const delStart = i
    while (i < lines.length && lines[i]?.type === 'del') {
      i++
    }
    const delEnd = i

    const addStart = i
    while (i < lines.length && lines[i]?.type === 'add') {
      i++
    }
    const addEnd = i

    const delCount = delEnd - delStart
    const addCount = addEnd - addStart

    // If we have matching del/add pairs, compute word diffs
    if (delCount > 0 && addCount > 0) {
      const pairCount = Math.min(delCount, addCount)
      for (let j = 0; j < pairCount; j++) {
        const delLine = lines[delStart + j]
        const addLine = lines[addStart + j]
        if (delLine && addLine) {
          const { oldChanges, newChanges } = computeWordDiff(delLine.content, addLine.content)
          delLine.changes = oldChanges
          addLine.changes = newChanges
        }
      }
    }

    // If we didn't move forward, advance by one
    if (i === delStart) {
      i++
    }
  }
}

/**
 * Detect programming language from filename extension
 */
function detectLanguage(filename: string): string | undefined {
  const ext = filename.split('.').pop()?.toLowerCase()

  const languageMap: Record<string, string> = {
    ts: 'typescript',
    tsx: 'tsx',
    js: 'javascript',
    jsx: 'jsx',
    py: 'python',
    rb: 'ruby',
    go: 'go',
    rs: 'rust',
    java: 'java',
    kt: 'kotlin',
    swift: 'swift',
    c: 'c',
    cpp: 'cpp',
    cc: 'cpp',
    h: 'c',
    hpp: 'cpp',
    cs: 'csharp',
    php: 'php',
    html: 'html',
    css: 'css',
    scss: 'scss',
    sass: 'sass',
    less: 'less',
    json: 'json',
    yaml: 'yaml',
    yml: 'yaml',
    xml: 'xml',
    md: 'markdown',
    sql: 'sql',
    sh: 'bash',
    bash: 'bash',
    zsh: 'bash',
    dockerfile: 'dockerfile',
    vue: 'vue',
    svelte: 'svelte',
  }

  return ext ? languageMap[ext] : undefined
}

/**
 * Get file status from diff metadata
 */
export function getFileStatus(
  oldFilename: string | undefined,
  newFilename: string | undefined,
): FileStatus {
  if (!oldFilename || oldFilename === '/dev/null') {
    return 'added'
  }
  if (!newFilename || newFilename === '/dev/null') {
    return 'deleted'
  }
  if (oldFilename !== newFilename) {
    return 'renamed'
  }
  return 'modified'
}

/**
 * Collapse consecutive unchanged (normal) lines into hidden chunks
 * Keeps contextLines visible before and after the collapsed section
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Collapsing logic requires multiple conditionals
function collapseUnchangedLines(
  lines: DiffLineData[],
  threshold: number,
  contextLines: number,
): DiffLineItem[] {
  const result: DiffLineItem[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]
    if (!line) {
      i++
      continue
    }

    // If not a normal line, just add it
    if (line.type !== 'normal') {
      result.push(line)
      i++
      continue
    }

    // Count consecutive normal lines
    const normalStart = i
    while (i < lines.length && lines[i]?.type === 'normal') {
      i++
    }
    const normalEnd = i
    const normalCount = normalEnd - normalStart

    // If below threshold, add all lines as-is
    if (normalCount < threshold) {
      for (let j = normalStart; j < normalEnd; j++) {
        const normalLine = lines[j]
        if (normalLine) {
          result.push(normalLine)
        }
      }
      continue
    }

    // Collapse: keep contextLines before, create hidden chunk, keep contextLines after
    const beforeCount = Math.min(contextLines, normalCount)
    const afterCount = Math.min(contextLines, normalCount - beforeCount)
    const hiddenCount = normalCount - beforeCount - afterCount

    // Add lines before the hidden chunk
    for (let j = normalStart; j < normalStart + beforeCount; j++) {
      const beforeLine = lines[j]
      if (beforeLine) {
        result.push(beforeLine)
      }
    }

    // Create hidden chunk if there are lines to hide
    if (hiddenCount > 0) {
      const hiddenLines: DiffLineData[] = []
      const hiddenStart = normalStart + beforeCount
      const hiddenEnd = hiddenStart + hiddenCount

      for (let j = hiddenStart; j < hiddenEnd; j++) {
        const hiddenLine = lines[j]
        if (hiddenLine) {
          hiddenLines.push(hiddenLine)
        }
      }

      // Get line numbers from the first hidden line
      const firstHiddenLine = lines[hiddenStart]
      const hiddenChunk: HiddenChunkData = {
        type: 'hidden-chunk',
        lineCount: hiddenCount,
        lines: hiddenLines,
        oldStart: firstHiddenLine?.oldNumber ?? 0,
        newStart: firstHiddenLine?.newNumber ?? 0,
      }
      result.push(hiddenChunk)
    }

    // Add lines after the hidden chunk
    for (let j = normalEnd - afterCount; j < normalEnd; j++) {
      const afterLine = lines[j]
      if (afterLine) {
        result.push(afterLine)
      }
    }
  }

  return result
}

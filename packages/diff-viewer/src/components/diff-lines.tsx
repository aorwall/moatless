import type { DiffLinesProps, HiddenChunkPosition } from '../types/index.js'
import { isHiddenChunk } from '../types/index.js'
import { DiffLine } from './diff-line.js'
import { HiddenChunk } from './hidden-chunk.js'

/**
 * Check if a line type is a visible content line (not hunk-header)
 */
function isVisibleLine(type: string): boolean {
  return type !== 'hunk-header'
}

/**
 * Determine the position of a hidden chunk based on surrounding VISIBLE lines
 * - 'start': No visible lines before this chunk (it's at the visual start)
 * - 'end': No visible lines after this chunk (it's at the visual end)
 * - 'middle': Visible lines exist both before and after
 */
function getChunkPosition(index: number, lines: DiffLinesProps['lines']): HiddenChunkPosition {
  // Check for visible lines BEFORE this chunk
  let hasVisibleLinesBefore = false
  for (let i = 0; i < index; i++) {
    const item = lines[i]
    if (item && !isHiddenChunk(item) && isVisibleLine(item.type)) {
      hasVisibleLinesBefore = true
      break
    }
  }

  // Check for visible lines AFTER this chunk
  let hasVisibleLinesAfter = false
  for (let i = index + 1; i < lines.length; i++) {
    const item = lines[i]
    if (item && !isHiddenChunk(item) && isVisibleLine(item.type)) {
      hasVisibleLinesAfter = true
      break
    }
  }

  // Determine position based on surrounding visible lines
  if (!hasVisibleLinesBefore && hasVisibleLinesAfter) {
    return 'start' // No visible lines before, expand down into the start
  }
  if (hasVisibleLinesBefore && !hasVisibleLinesAfter) {
    return 'end' // No visible lines after, expand up into the end
  }
  // Visible lines on both sides
  return 'middle'
}

/**
 * Renders an array of diff lines
 * Supports unified view mode (split view to be added in future)
 * Handles hidden chunks with position-aware expansion:
 * - start: expand down
 * - end: expand up
 * - middle: expand all
 */
export function DiffLines({
  lines,
  language,
  chunkExpansion,
  onExpandTop,
  onExpandBottom,
  onExpandAll,
}: DiffLinesProps) {
  if (lines.length === 0) {
    return (
      <div className="flex min-h-[48px] items-center justify-center text-sm text-muted-foreground">
        No changes
      </div>
    )
  }

  return (
    <div className="diff-lines">
      {lines.map((item, index) => {
        // Handle hidden chunks
        if (isHiddenChunk(item)) {
          const expansion = chunkExpansion?.get(index) ?? {
            revealedFromTop: 0,
            revealedFromBottom: 0,
          }
          const { revealedFromTop, revealedFromBottom } = expansion
          const position = getChunkPosition(index, lines)

          // Use position-based key for hidden chunks (unique based on line numbers)
          const chunkKey = `hidden-${item.oldStart}-${item.newStart}`

          // For 'start' position: show lines from top, expand downward
          // For 'end' position: show lines from bottom, expand upward
          // For 'middle' position: show all lines when expanded
          let topLines: typeof item.lines = []
          let bottomLines: typeof item.lines = []

          if (position === 'start') {
            // Reveal from top downward
            topLines = item.lines.slice(0, revealedFromTop)
          } else if (position === 'end') {
            // Reveal from bottom upward
            const bottomStartIndex = item.lines.length - revealedFromBottom
            bottomLines = revealedFromBottom > 0 ? item.lines.slice(bottomStartIndex) : []
          } else {
            // Middle: when expanded, show all lines (revealedFromTop tracks full expansion)
            topLines = item.lines.slice(0, revealedFromTop)
          }

          return (
            <div key={chunkKey}>
              {/* Render revealed top lines (for 'start' and 'middle' positions) */}
              {topLines.map((line, lineIdx) => {
                const lineKey = `${chunkKey}-top-${line.oldNumber ?? 'x'}-${line.newNumber ?? 'x'}-${lineIdx}`
                return <DiffLine key={lineKey} line={line} language={language} />
              })}

              {/* Hidden chunk control (hides when fully revealed) */}
              <HiddenChunk
                lineCount={item.lineCount}
                position={position}
                revealedFromTop={revealedFromTop}
                revealedFromBottom={revealedFromBottom}
                onExpandTop={(count) => onExpandTop?.(index, count)}
                onExpandBottom={(count) => onExpandBottom?.(index, count)}
                onExpandAll={() => onExpandAll?.(index, item.lineCount)}
              />

              {/* Render revealed bottom lines (for 'end' position) */}
              {bottomLines.map((line, lineIdx) => {
                const lineKey = `${chunkKey}-bottom-${line.oldNumber ?? 'x'}-${line.newNumber ?? 'x'}-${lineIdx}`
                return <DiffLine key={lineKey} line={line} language={language} />
              })}
            </div>
          )
        }

        // Regular diff line
        const key = `${item.type}-${item.oldNumber ?? 'x'}-${item.newNumber ?? 'x'}-${index}`
        return <DiffLine key={key} line={item} language={language} />
      })}
    </div>
  )
}

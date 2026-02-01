import type { DiffLinesProps, HiddenChunkPosition } from '../types/diff'
import { isHiddenChunk } from '../types/diff'
import { DiffLine } from './diff-line'
import { HiddenChunk } from './hidden-chunk'

/**
 * Check if a line type is a visible content line (not hunk-header)
 */
function isVisibleLine(type: string): boolean {
  return type !== 'hunk-header'
}

/**
 * Determine the position of a hidden chunk based on surrounding visible lines
 */
function getChunkPosition(index: number, lines: DiffLinesProps['lines']): HiddenChunkPosition {
  let hasVisibleLinesBefore = false
  for (let i = 0; i < index; i++) {
    const item = lines[i]
    if (item && !isHiddenChunk(item) && isVisibleLine(item.type)) {
      hasVisibleLinesBefore = true
      break
    }
  }

  let hasVisibleLinesAfter = false
  for (let i = index + 1; i < lines.length; i++) {
    const item = lines[i]
    if (item && !isHiddenChunk(item) && isVisibleLine(item.type)) {
      hasVisibleLinesAfter = true
      break
    }
  }

  if (!hasVisibleLinesBefore && hasVisibleLinesAfter) {
    return 'start'
  }
  if (hasVisibleLinesBefore && !hasVisibleLinesAfter) {
    return 'end'
  }
  return 'middle'
}

/**
 * Renders an array of diff lines with hidden chunk expansion support
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
      <div className="flex min-h-[48px] items-center justify-center text-sm" style={{ color: 'var(--muted-foreground)' }}>
        No changes
      </div>
    )
  }

  return (
    <div className="diff-lines">
      {lines.map((item, index) => {
        if (isHiddenChunk(item)) {
          const expansion = chunkExpansion?.get(index) ?? {
            revealedFromTop: 0,
            revealedFromBottom: 0,
          }
          const { revealedFromTop, revealedFromBottom } = expansion
          const position = getChunkPosition(index, lines)

          const chunkKey = `hidden-${item.oldStart}-${item.newStart}`

          let topLines: typeof item.lines = []
          let bottomLines: typeof item.lines = []

          if (position === 'start') {
            topLines = item.lines.slice(0, revealedFromTop)
          } else if (position === 'end') {
            const bottomStartIndex = item.lines.length - revealedFromBottom
            bottomLines = revealedFromBottom > 0 ? item.lines.slice(bottomStartIndex) : []
          } else {
            topLines = item.lines.slice(0, revealedFromTop)
          }

          return (
            <div key={chunkKey}>
              {topLines.map((line, lineIdx) => {
                const lineKey = `${chunkKey}-top-${line.oldNumber ?? 'x'}-${line.newNumber ?? 'x'}-${lineIdx}`
                return <DiffLine key={lineKey} line={line} language={language} />
              })}

              <HiddenChunk
                lineCount={item.lineCount}
                position={position}
                revealedFromTop={revealedFromTop}
                revealedFromBottom={revealedFromBottom}
                onExpandTop={(count) => onExpandTop?.(index, count)}
                onExpandBottom={(count) => onExpandBottom?.(index, count)}
                onExpandAll={() => onExpandAll?.(index, item.lineCount)}
              />

              {bottomLines.map((line, lineIdx) => {
                const lineKey = `${chunkKey}-bottom-${line.oldNumber ?? 'x'}-${line.newNumber ?? 'x'}-${lineIdx}`
                return <DiffLine key={lineKey} line={line} language={language} />
              })}
            </div>
          )
        }

        const key = `${item.type}-${item.oldNumber ?? 'x'}-${item.newNumber ?? 'x'}-${index}`
        return <DiffLine key={key} line={item} language={language} />
      })}
    </div>
  )
}

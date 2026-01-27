import { useCallback, useState } from 'react'
import type { ChunkExpansionState, DiffFileCardProps } from '../types/index.js'
import { DiffFileHeader } from './diff-file-header.js'
import { DiffLines } from './diff-lines.js'

/**
 * Renders a collapsible card containing a file's diff
 * Includes header with file info and collapsible body with diff content
 * Manages hidden chunk expansion state internally with support for partial expansion
 */
export function DiffFileCard({
  file,
  collapsed: controlledCollapsed,
  onCollapseChange,
}: DiffFileCardProps) {
  const [internalCollapsed, setInternalCollapsed] = useState(false)
  const [chunkExpansion, setChunkExpansion] = useState<Map<number, ChunkExpansionState>>(new Map())

  // Support both controlled and uncontrolled modes
  const isCollapsed = controlledCollapsed ?? internalCollapsed
  const handleCollapseChange = (newCollapsed: boolean) => {
    if (onCollapseChange) {
      onCollapseChange(newCollapsed)
    } else {
      setInternalCollapsed(newCollapsed)
    }
  }

  // Handle expanding N lines from top of a hidden chunk
  const handleExpandTop = useCallback((chunkIndex: number, count: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      const current = next.get(chunkIndex) ?? { revealedFromTop: 0, revealedFromBottom: 0 }
      next.set(chunkIndex, {
        ...current,
        revealedFromTop: current.revealedFromTop + count,
      })
      return next
    })
  }, [])

  // Handle expanding N lines from bottom of a hidden chunk
  const handleExpandBottom = useCallback((chunkIndex: number, count: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      const current = next.get(chunkIndex) ?? { revealedFromTop: 0, revealedFromBottom: 0 }
      next.set(chunkIndex, {
        ...current,
        revealedFromBottom: current.revealedFromBottom + count,
      })
      return next
    })
  }, [])

  // Handle expanding all lines in a hidden chunk
  const handleExpandAll = useCallback((chunkIndex: number, totalLines: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      // Set revealedFromTop to total lines to fully expand
      next.set(chunkIndex, {
        revealedFromTop: totalLines,
        revealedFromBottom: 0,
      })
      return next
    })
  }, [])

  // Flatten all lines from all hunks
  const allLines = file.hunks.flatMap((hunk) => hunk.lines)

  return (
    <div
      className="diff-file-card overflow-hidden rounded-lg border border-border"
      id={`diff-${file.filename.replace(/[^a-zA-Z0-9]/g, '-')}`}
    >
      <DiffFileHeader file={file} collapsed={isCollapsed} onCollapseChange={handleCollapseChange} />
      {!isCollapsed && (
        <div className="diff-file-content overflow-x-auto">
          <DiffLines
            lines={allLines}
            language={file.language}
            chunkExpansion={chunkExpansion}
            onExpandTop={handleExpandTop}
            onExpandBottom={handleExpandBottom}
            onExpandAll={handleExpandAll}
          />
        </div>
      )}
    </div>
  )
}

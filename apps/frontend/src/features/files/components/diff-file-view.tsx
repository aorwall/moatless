import {
  type ChunkExpansionState,
  type DiffLineItem,
  DiffLines,
  parseDiff,
} from '@moatless/diff-viewer'
import { createTwoFilesPatch } from 'diff'
import { useMemo, useState } from 'react'

interface DiffFileViewProps {
  oldContent: string
  newContent: string
  filename: string
}

export function DiffFileView({ oldContent, newContent, filename }: DiffFileViewProps) {
  const [chunkExpansion, setChunkExpansion] = useState<Map<number, ChunkExpansionState>>(new Map())

  const { lines, language } = useMemo(() => {
    const patch = createTwoFilesPatch(filename, filename, oldContent, newContent)
    const parsed = parseDiff(`diff --git a/${filename} b/${filename}\n${patch}`)
    const file = parsed.files[0]
    if (!file) {
      return { lines: [] as DiffLineItem[], language: undefined }
    }

    const allLines: DiffLineItem[] = []
    for (const hunk of file.hunks) {
      for (const line of hunk.lines) {
        allLines.push(line)
      }
    }

    return { lines: allLines, language: file.language }
  }, [oldContent, newContent, filename])

  const handleExpandTop = (chunkIndex: number, count: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      const current = next.get(chunkIndex) ?? { revealedFromTop: 0, revealedFromBottom: 0 }
      next.set(chunkIndex, { ...current, revealedFromTop: current.revealedFromTop + count })
      return next
    })
  }

  const handleExpandBottom = (chunkIndex: number, count: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      const current = next.get(chunkIndex) ?? { revealedFromTop: 0, revealedFromBottom: 0 }
      next.set(chunkIndex, { ...current, revealedFromBottom: current.revealedFromBottom + count })
      return next
    })
  }

  const handleExpandAll = (chunkIndex: number, totalLines: number) => {
    setChunkExpansion((prev) => {
      const next = new Map(prev)
      next.set(chunkIndex, { revealedFromTop: totalLines, revealedFromBottom: 0 })
      return next
    })
  }

  return (
    <div className="diff-file-view overflow-auto">
      <DiffLines
        lines={lines}
        language={language}
        chunkExpansion={chunkExpansion}
        onExpandTop={handleExpandTop}
        onExpandBottom={handleExpandBottom}
        onExpandAll={handleExpandAll}
      />
    </div>
  )
}

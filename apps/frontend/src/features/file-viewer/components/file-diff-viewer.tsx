import { useMemo } from 'react'
import { Diff, Hunk, parseDiff } from 'react-diff-view'
import 'react-diff-view/style/index.css'
import { tokenizeHunks } from '@/features/tasks/components/git-diff-viewer/syntax-highlighter'

interface FileDiffViewerProps {
  filePath: string
  oldContent: string
  newContent: string
  additions?: number
  deletions?: number
}

export function FileDiffViewer({
  filePath,
  oldContent,
  newContent,
  additions = 0,
  deletions = 0,
}: FileDiffViewerProps) {
  const { parsedDiff } = useMemo(() => {
    // Create a unified diff format
    const oldLines = oldContent.split('\n')
    const newLines = newContent.split('\n')

    // Create a simple unified diff header
    const header = [
      `--- a/${filePath}`,
      `+++ b/${filePath}`,
      `@@ -1,${oldLines.length} +1,${newLines.length} @@`,
    ].join('\n')

    // Create the diff content
    const diffLines: string[] = [header]

    // Simple diff algorithm - this is a placeholder
    // In production, you'd want to use a proper diff algorithm
    const maxLines = Math.max(oldLines.length, newLines.length)
    for (let i = 0; i < maxLines; i++) {
      const oldLine = oldLines[i]
      const newLine = newLines[i]

      if (oldLine === newLine && oldLine !== undefined) {
        diffLines.push(` ${oldLine}`)
      } else {
        if (oldLine !== undefined && i < oldLines.length) {
          diffLines.push(`-${oldLine}`)
        }
        if (newLine !== undefined && i < newLines.length) {
          diffLines.push(`+${newLine}`)
        }
      }
    }

    const diffText = diffLines.join('\n')

    // Parse the diff
    const files = parseDiff(diffText)
    return { parsedDiff: files[0] }
  }, [filePath, oldContent, newContent])

  // Get syntax highlighting tokens
  const tokens = useMemo(() => {
    if (!parsedDiff?.hunks) return undefined
    return tokenizeHunks(parsedDiff.hunks, filePath)
  }, [parsedDiff, filePath])

  if (!parsedDiff) {
    return <div>Unable to generate diff</div>
  }

  return (
    <div className="file-diff-viewer h-full overflow-auto">
      <div className="diff-stats p-4 border-b text-sm">
        <span className="text-status-success">+{additions} additions</span>
        <span className="mx-2">·</span>
        <span className="text-status-error">−{deletions} deletions</span>
      </div>
      <div className="diff-content">
        <Diff
          viewType="unified"
          diffType={parsedDiff.type || 'unified'}
          hunks={parsedDiff.hunks}
          className="diff-view"
          tokens={tokens}
          optimizeSelection
          renderToken={(token, defaultRender) => {
            // Handle prism tokens with className for syntax highlighting
            const prismToken = token as { type?: string; className?: string; value?: string }
            if (prismToken.type === 'prism' && prismToken.className) {
              return <span className={prismToken.className}>{prismToken.value || ''}</span>
            }
            return defaultRender(token, 0)
          }}
        >
          {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
        </Diff>
      </div>
    </div>
  )
}

import type { ReactNode } from 'react'
import { useMemo } from 'react'
import { Diff, Hunk, parseDiff } from 'react-diff-view'
import 'react-diff-view/style/index.css'

interface DiffViewProps {
  diffContent: string
  fallback?: ReactNode
}

export function DiffView({ diffContent, fallback }: DiffViewProps) {
  // Parse the diff
  const parsedFiles = useMemo(() => {
    try {
      return parseDiff(diffContent)
    } catch (error) {
      console.error('Failed to parse diff:', error)
      return []
    }
  }, [diffContent])

  const file = parsedFiles[0]

  if (!file && fallback) {
    return <>{fallback}</>
  }

  if (!file) {
    return (
      <div className="text-xs text-muted-foreground p-3 bg-muted rounded">Failed to parse diff</div>
    )
  }

  return (
    <div className="github-diff-file border border-border rounded-lg overflow-hidden">
      <div className="github-diff-content overflow-auto" style={{ maxHeight: '20rem' }}>
        <Diff viewType="unified" diffType={file.type} hunks={file.hunks} className="diff-view">
          {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
        </Diff>
      </div>
    </div>
  )
}

import { FilePlus } from 'lucide-react'
import type { ReactNode } from 'react'
import { useMemo } from 'react'

import { DiffView } from './shared/diff-view'

interface WriteToolCallProps {
  input: {
    filePath: string
    content: string
  }
  name: string
}

export function WriteToolCall({ input, name }: WriteToolCallProps): ReactNode {
  // Ensure content is a string or empty string for the hooks
  const content =
    input?.content !== undefined && input?.content !== null ? String(input.content) : ''
  const filePath = input?.filePath || ''

  // Create a unified diff format showing only additions (new file)
  const diffContent = useMemo(() => {
    if (!filePath || !content) return ''

    // Extract just the filename from the path
    const fileName = filePath.split('/').pop() || filePath

    // Create a unified diff format for a new file (all additions)
    const lines = content.split('\n')
    const diff = [
      `--- /dev/null`,
      `+++ b/${fileName}`,
      `@@ -0,0 +1,${lines.length} @@`,
      ...lines.map((line) => `+ ${line}`),
    ].join('\n')

    return diff
  }, [filePath, content])

  // Count lines for display
  const lineCount = useMemo(() => {
    return content ? content.split('\n').length : 0
  }, [content])

  // Validate input after hooks
  if (!input || typeof input !== 'object') {
    console.error('[WriteToolCall] Invalid input:', input)
    return <div className="text-sm text-destructive">Error: Invalid input for {name} tool</div>
  }

  if (!input.filePath || typeof input.filePath !== 'string') {
    console.error('[WriteToolCall] Missing or invalid filePath:', input)
    return <div className="text-sm text-destructive">Error: Missing file path for {name} tool</div>
  }

  if (input.content === undefined || input.content === null) {
    console.error('[WriteToolCall] Missing content:', input)
    return <div className="text-sm text-destructive">Error: Missing content for {name} tool</div>
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FilePlus className="h-3.5 w-3.5 text-status-success" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">- {input.filePath}</span>
        <span className="text-xs text-muted-foreground">({lineCount} lines)</span>
      </div>

      <DiffView
        diffContent={diffContent}
        fallback={
          <div className="bg-status-success/10 rounded p-2">
            <span className="text-xs text-status-success font-mono">+ New file:</span>
            <pre className="text-xs font-mono whitespace-pre-wrap mt-1">{content}</pre>
          </div>
        }
      />
    </div>
  )
}

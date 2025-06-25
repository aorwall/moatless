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
  // Validate input
  if (!input || typeof input !== 'object') {
    console.error('[WriteToolCall] Invalid input:', input)
    return (
      <div className="text-sm text-red-600 dark:text-red-400">
        Error: Invalid input for {name} tool
      </div>
    )
  }

  if (!input.filePath || typeof input.filePath !== 'string') {
    console.error('[WriteToolCall] Missing or invalid filePath:', input)
    return (
      <div className="text-sm text-red-600 dark:text-red-400">
        Error: Missing file path for {name} tool
      </div>
    )
  }

  if (input.content === undefined || input.content === null) {
    console.error('[WriteToolCall] Missing content:', input)
    return (
      <div className="text-sm text-red-600 dark:text-red-400">
        Error: Missing content for {name} tool
      </div>
    )
  }

  // Ensure content is a string
  const content = String(input.content)

  // Create a unified diff format showing only additions (new file)
  const diffContent = useMemo(() => {
    // Extract just the filename from the path
    const fileName = input.filePath.split('/').pop() || input.filePath

    // Create a unified diff format for a new file (all additions)
    const lines = content.split('\n')
    const diff = [
      `--- /dev/null`,
      `+++ b/${fileName}`,
      `@@ -0,0 +1,${lines.length} @@`,
      ...lines.map((line) => `+ ${line}`),
    ].join('\n')

    return diff
  }, [input.filePath, content])

  // Count lines for display
  const lineCount = useMemo(() => {
    return content.split('\n').length
  }, [content])

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FilePlus className="h-3.5 w-3.5 text-green-600 dark:text-green-400" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">- {input.filePath}</span>
        <span className="text-xs text-muted-foreground">({lineCount} lines)</span>
      </div>

      <DiffView
        diffContent={diffContent}
        fallback={
          <div className="bg-green-50 dark:bg-green-900/20 rounded p-2">
            <span className="text-xs text-green-600 dark:text-green-400 font-mono">
              + New file:
            </span>
            <pre className="text-xs font-mono whitespace-pre-wrap mt-1">{content}</pre>
          </div>
        }
      />
    </div>
  )
}

import { FileEdit } from 'lucide-react'
import type { ReactNode } from 'react'
import { useMemo } from 'react'

import { DiffView } from './shared/diff-view'

interface EditToolCallProps {
  input: {
    filePath: string
    oldString: string
    newString: string
    replaceAll?: boolean
  }
  name: string
}

export function EditToolCall({ input, name }: EditToolCallProps): ReactNode {
  // Create a unified diff format from the old and new strings
  const diffContent = useMemo(() => {
    // Extract just the filename from the path
    const fileName = input.filePath.split('/').pop() || input.filePath

    // Create a unified diff format
    const diff = [
      `--- a/${fileName}`,
      `+++ b/${fileName}`,
      '@@ -1,1 +1,1 @@',
      ...input.oldString.split('\n').map((line) => `- ${line}`),
      ...input.newString.split('\n').map((line) => `+ ${line}`),
    ].join('\n')

    return diff
  }, [input.filePath, input.oldString, input.newString])

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FileEdit className="h-3.5 w-3.5 text-status-info" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">- {input.filePath}</span>
        {input.replaceAll && (
          <span className="text-xs bg-status-warning/10 text-status-warning px-1.5 py-0.5 rounded">
            Replace All
          </span>
        )}
      </div>

      <DiffView
        diffContent={diffContent}
        fallback={
          <div className="space-y-2 text-xs">
            <div className="bg-status-error/10 rounded p-2 font-mono">
              <span className="text-status-error">- Old:</span>
              <pre className="whitespace-pre-wrap mt-1">{input.oldString}</pre>
            </div>
            <div className="bg-status-success/10 rounded p-2 font-mono">
              <span className="text-status-success">+ New:</span>
              <pre className="whitespace-pre-wrap mt-1">{input.newString}</pre>
            </div>
          </div>
        }
      />
    </div>
  )
}

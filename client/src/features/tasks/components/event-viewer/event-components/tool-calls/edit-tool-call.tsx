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
        <FileEdit className="h-3.5 w-3.5 text-blue-600 dark:text-blue-400" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">- {input.filePath}</span>
        {input.replaceAll && (
          <span className="text-xs bg-yellow-100 dark:bg-yellow-900/30 text-yellow-800 dark:text-yellow-300 px-1.5 py-0.5 rounded">
            Replace All
          </span>
        )}
      </div>

      <DiffView
        diffContent={diffContent}
        fallback={
          <div className="space-y-2 text-xs">
            <div className="bg-red-50 dark:bg-red-900/20 rounded p-2 font-mono">
              <span className="text-red-600 dark:text-red-400">- Old:</span>
              <pre className="whitespace-pre-wrap mt-1">{input.oldString}</pre>
            </div>
            <div className="bg-green-50 dark:bg-green-900/20 rounded p-2 font-mono">
              <span className="text-green-600 dark:text-green-400">+ New:</span>
              <pre className="whitespace-pre-wrap mt-1">{input.newString}</pre>
            </div>
          </div>
        }
      />
    </div>
  )
}

import { FolderSearch } from 'lucide-react'
import type { ReactNode } from 'react'

interface GlobToolCallProps {
  input: {
    pattern: string
    path?: string
  }
  name: string
}

export function GlobToolCall({ input, name }: GlobToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FolderSearch className="h-3.5 w-3.5 text-status-info" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-background border rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-status-success select-none">$</span>
          <code className="text-foreground break-all whitespace-pre-wrap">
            find {input.path || '.'} -name "{input.pattern}"
          </code>
        </div>
      </div>
    </div>
  )
}

import { FolderOpen } from 'lucide-react'
import type { ReactNode } from 'react'

interface LSToolCallProps {
  input: {
    path: string
    ignore?: string[]
  }
  name: string
}

export function LSToolCall({ input, name }: LSToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FolderOpen className="h-3.5 w-3.5 text-status-info" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-background border rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-status-success select-none">$</span>
          <code className="text-foreground break-all whitespace-pre-wrap">ls {input.path}</code>
        </div>
      </div>

      {input.ignore && input.ignore.length > 0 && (
        <div className="text-xs text-muted-foreground">Ignoring: {input.ignore.join(', ')}</div>
      )}
    </div>
  )
}

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
        <FolderOpen className="h-3.5 w-3.5 text-sky-600 dark:text-sky-400" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-zinc-900 dark:bg-zinc-950 rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-green-400 select-none">$</span>
          <code className="text-zinc-100 break-all whitespace-pre-wrap">ls {input.path}</code>
        </div>
      </div>

      {input.ignore && input.ignore.length > 0 && (
        <div className="text-xs text-muted-foreground">Ignoring: {input.ignore.join(', ')}</div>
      )}
    </div>
  )
}

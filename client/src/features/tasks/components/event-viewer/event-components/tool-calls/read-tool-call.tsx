import { FileText } from 'lucide-react'
import type { ReactNode } from 'react'

interface ReadToolCallProps {
  input: {
    file_path: string
    offset?: number
    limit?: number
  }
  name: string
}

export function ReadToolCall({ input, name }: ReadToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FileText className="h-3.5 w-3.5 text-purple-600 dark:text-purple-400" />
        <span className="font-medium text-foreground">{name}</span>
        {(input.offset !== undefined || input.limit !== undefined) && (
          <span className="text-xs text-muted-foreground">
            {input.offset !== undefined && `L${input.offset}`}
            {input.offset !== undefined && input.limit !== undefined && '-'}
            {input.limit !== undefined && `L${(input.offset || 0) + input.limit}`}
          </span>
        )}
      </div>

      <div className="bg-zinc-900 dark:bg-zinc-950 rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-green-400 select-none">$</span>
          <code className="text-zinc-100 break-all whitespace-pre-wrap">cat {input.file_path}</code>
        </div>
      </div>
    </div>
  )
}

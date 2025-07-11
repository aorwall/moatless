import { FileText } from 'lucide-react'
import type { ReactNode } from 'react'

interface ReadToolCallProps {
  input: {
    filePath: string
    offset?: number
    limit?: number
  }
  name: string
}

export function ReadToolCall({ input, name }: ReadToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FileText className="h-3.5 w-3.5 text-status-info" />
        <span className="font-medium text-foreground">{name}</span>
        {(input.offset !== undefined || input.limit !== undefined) && (
          <span className="text-xs text-muted-foreground">
            {input.offset !== undefined && `L${input.offset}`}
            {input.offset !== undefined && input.limit !== undefined && '-'}
            {input.limit !== undefined && `L${(input.offset || 0) + input.limit}`}
          </span>
        )}
      </div>

      <div className="bg-background border rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-status-success select-none">$</span>
          <code className="text-foreground break-all whitespace-pre-wrap">
            cat {input.filePath}
          </code>
        </div>
      </div>
    </div>
  )
}

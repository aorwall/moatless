import { Search } from 'lucide-react'
import type { ReactNode } from 'react'

interface GrepToolCallProps {
  input: {
    pattern: string
    path?: string
    include?: string
  }
  name: string
}

export function GrepToolCall({ input, name }: GrepToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Search className="h-3.5 w-3.5 text-status-warning" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-background border rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-status-success select-none">$</span>
          <code className="text-foreground break-all whitespace-pre-wrap">
            grep "{input.pattern}"{input.path ? ` ${input.path}` : ''}
            {input.include ? ` --include="${input.include}"` : ''}
          </code>
        </div>
      </div>
    </div>
  )
}

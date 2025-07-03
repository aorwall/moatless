import { Terminal } from 'lucide-react'
import type { ReactNode } from 'react'

interface BashToolCallProps {
  input: {
    command: string
    description?: string
    timeout?: number
  }
  name: string
}

export function BashToolCall({ input, name }: BashToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Terminal className="h-3.5 w-3.5 text-status-success" />
        <span className="font-medium text-foreground">{name}</span>
        {input.description && (
          <span className="text-xs text-muted-foreground">- {input.description}</span>
        )}
      </div>

      {/* Terminal-style command display */}
      <div className="bg-background border rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-status-success select-none">$</span>
          <code className="text-foreground break-all whitespace-pre-wrap">{input.command}</code>
        </div>
      </div>

      {input.timeout && (
        <div className="text-xs text-muted-foreground">
          Timeout: {(input.timeout / 1000).toFixed(1)}s
        </div>
      )}
    </div>
  )
}

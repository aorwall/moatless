import { Brain } from 'lucide-react'
import type { ReactNode } from 'react'

interface TaskToolCallProps {
  input: {
    description: string
    prompt: string
  }
  name: string
}

export function TaskToolCall({ input, name }: TaskToolCallProps): ReactNode {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Brain className="h-3.5 w-3.5 text-violet-600 dark:text-violet-400" />
        <span className="font-medium text-foreground">{name}</span>
        {input.description && (
          <>
            <span className="text-xs text-muted-foreground">-</span>
            <span className="text-sm text-muted-foreground">{input.description}</span>
          </>
        )}
      </div>

      {input.prompt && (
        <div className="bg-zinc-900 dark:bg-zinc-950 rounded-lg p-3 font-mono text-sm">
          <code className="text-zinc-100 break-words whitespace-pre-wrap">{input.prompt}</code>
        </div>
      )}
    </div>
  )
}

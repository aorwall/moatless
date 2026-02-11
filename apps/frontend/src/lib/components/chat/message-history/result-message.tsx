import type { UiMessage } from '@moatless/api-client'
import { AlertCircle, Square } from 'lucide-react'
import { cn } from '@/lib/utils'

interface ResultMessageProps {
  message: UiMessage
}

function ResultMessageComponent({ message }: ResultMessageProps) {
  const status = message.result?.turnStatus

  // Success results are silent — don't show anything
  if (status === 'completed') {
    return null
  }

  const isError = status === 'error'
  const text = message.content?.[0]?.text

  return (
    <div
      className={cn(
        'rounded-lg px-4 py-3 flex items-start gap-2',
        isError
          ? 'bg-destructive/10 border border-destructive/20'
          : 'bg-muted/50 border border-border/50',
      )}
    >
      {isError ? (
        <AlertCircle className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
      ) : (
        <Square className="h-4 w-4 text-muted-foreground mt-0.5 shrink-0" />
      )}
      <div className="text-sm">
        {isError && <span className="font-medium text-destructive">Agent Error</span>}
        {status === 'stopped' && (
          <span className="font-medium text-muted-foreground">Agent Stopped</span>
        )}
        {text && <p className="text-muted-foreground mt-0.5">{text}</p>}
      </div>
    </div>
  )
}

export { ResultMessageComponent }

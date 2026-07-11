import type { UiMessage } from '@moatless/api-client'
import { Clock, X } from 'lucide-react'

interface QueuedMessagesProps {
  messages: UiMessage[]
  onRemove: (messageId: string) => void
}

// Extract text content from UiMessage content blocks
function getTextContent(message: UiMessage): string {
  return message.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text ?? '')
    .join('\n')
}

export function QueuedMessages({ messages, onRemove }: QueuedMessagesProps) {
  if (messages.length === 0) {
    return null
  }

  return (
    <div className="mx-4 mb-2 flex flex-col gap-1">
      {messages.map((message) => (
        <div
          key={message.uuid}
          className="flex items-center gap-2 px-3 py-1.5 rounded-md border border-border/50 bg-muted/30 text-sm"
        >
          <Clock className="h-3 w-3 text-muted-foreground shrink-0" />
          <span className="text-muted-foreground text-xs font-medium shrink-0">Queued</span>
          <span className="truncate text-foreground/80 flex-1 min-w-0">
            {getTextContent(message)}
          </span>
          <button
            type="button"
            onClick={() => onRemove(message.uuid)}
            className="shrink-0 p-0.5 rounded hover:bg-muted-foreground/20 text-muted-foreground hover:text-foreground transition-colors"
            aria-label="Remove queued message"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  )
}

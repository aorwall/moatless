import type { UiMessage } from '@moatless/api-client'
import { Clock } from 'lucide-react'

interface QueuedMessagesProps {
  messages: UiMessage[]
}

// Extract text content from UiMessage content blocks
function getTextContent(message: UiMessage): string {
  return message.content.flatMap((c) => (c.type === 'text' ? [c.text ?? ''] : [])).join('\n')
}

/**
 * View-only list of messages queued behind a running turn. The cancel
 * affordance was removed with the `DELETE /messages` route — the pending list
 * is informational only.
 */
export function QueuedMessages({ messages }: QueuedMessagesProps) {
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
        </div>
      ))}
    </div>
  )
}

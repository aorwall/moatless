import type { UiMessage } from '@moatless/api-client'
import { ChevronDown, ChevronRight, Eye, FileText, Terminal } from 'lucide-react'
import { useState } from 'react'
import { ContextItems } from '@/features/tasks/components/chat/chat-input/context-items'
import { Message, MessageContent } from '@/lib/components/ai-elements/message'
import { Response } from '@/lib/components/ai-elements/response'
import { cn } from '@/lib/utils'

interface UserMessageProps {
  message: UiMessage
  onPreviewUrlClick?: ((path: string) => void) | undefined
}

// Extract text content from UiMessage content blocks
function getTextContent(message: UiMessage): string {
  return message.content.flatMap((c) => (c.type === 'text' ? [c.text ?? ''] : [])).join('\n')
}

export function UserMessageComponent({ message, onPreviewUrlClick }: UserMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = (message.messageType as string) === 'transcript_only'
  const isSummary = (message.messageType as string) === 'compact_summary'
  const textContent = getTextContent(message)

  const contexts = message.contexts ?? []

  // For hidden messages, show collapsed view by default
  if (isHidden) {
    return (
      <div
        className={cn(
          'border border-dashed rounded-lg overflow-hidden',
          isSummary
            ? 'border-purple-500/30 bg-purple-500/5'
            : 'border-muted-foreground/30 bg-muted/20',
        )}
      >
        <button
          type="button"
          onClick={() => setIsExpanded(!isExpanded)}
          className="w-full flex items-center gap-2 p-2 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
        >
          {isExpanded ? (
            <ChevronDown className="h-3 w-3 flex-shrink-0" />
          ) : (
            <ChevronRight className="h-3 w-3 flex-shrink-0" />
          )}
          {isSummary ? (
            <>
              <FileText className="h-3 w-3 flex-shrink-0 text-purple-500" />
              <span className="font-medium">Summary Message</span>
            </>
          ) : (
            <>
              <Eye className="h-3 w-3 flex-shrink-0" />
              <span className="font-medium">Hidden Message</span>
            </>
          )}
          <span className="text-muted-foreground/70 truncate">
            {textContent.slice(0, 50)}
            {textContent.length > 50 ? '...' : ''}
          </span>
        </button>

        {isExpanded && (
          <div className="p-3 border-t border-dashed border-muted-foreground/20">
            <div className="text-sm opacity-80">
              <Response>{textContent}</Response>
            </div>
            {contexts.length > 0 && (
              <div className="mt-2">
                <ContextItems contexts={contexts} onPreviewUrlClick={onPreviewUrlClick} />
              </div>
            )}
          </div>
        )}
      </div>
    )
  }

  // Normal message display
  return (
    <Message from="user">
      <MessageContent variant="flat">
        {message.commandName && (
          <span className="inline-flex items-center gap-1 rounded-md bg-secondary px-2 py-0.5 text-xs font-medium text-muted-foreground mb-1">
            <Terminal className="h-3 w-3" />/{message.commandName}
          </span>
        )}
        <div className="max-w-none break-words">{textContent}</div>
        {contexts.length > 0 && (
          <ContextItems contexts={contexts} onPreviewUrlClick={onPreviewUrlClick} />
        )}
      </MessageContent>
    </Message>
  )
}

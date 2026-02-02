import type { UserMessage } from '@moatless/types'
import { ChevronDown, ChevronRight, Eye, FileText } from 'lucide-react'
import { useState } from 'react'
import { Message, MessageContent } from '@/lib/components/ai-elements/message'
import { Response } from '@/lib/components/ai-elements/response'
import { ContextItems } from '@/lib/components/chat/chat-input/context-items'
import { FileAttachments } from '@/lib/components/chat/chat-input/file-attachments'
import { cn } from '@/lib/utils'

interface UserMessageProps {
  message: UserMessage
  onPreviewUrlClick?: (path: string) => void
}

export function UserMessageComponent({ message, onPreviewUrlClick }: UserMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = message.isVisibleInTranscriptOnly
  const isSummary = message.isCompactSummary

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
            {message.content.slice(0, 50)}
            {message.content.length > 50 ? '...' : ''}
          </span>
        </button>

        {isExpanded && (
          <div className="p-3 border-t border-dashed border-muted-foreground/20">
            <div className="text-sm opacity-80">
              <Response>{message.content}</Response>
            </div>
            {message.contexts && message.contexts.length > 0 && (
              <div className="mt-2">
                <ContextItems contexts={message.contexts} onPreviewUrlClick={onPreviewUrlClick} />
              </div>
            )}
            {message.attachments && message.attachments.length > 0 && (
              <div className="mt-2">
                <FileAttachments files={message.attachments} />
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
        <div className="max-w-none break-words">{message.content}</div>
        {message.contexts && message.contexts.length > 0 && (
          <ContextItems contexts={message.contexts} onPreviewUrlClick={onPreviewUrlClick} />
        )}
        {message.attachments && message.attachments.length > 0 && (
          <FileAttachments files={message.attachments} />
        )}
      </MessageContent>
    </Message>
  )
}

import type { UiMessage } from '@moatless/api-client'
import { ChevronDown, ChevronRight, Settings } from 'lucide-react'
import { useState } from 'react'
import { Response } from '@/lib/components/ai-elements/response'
import { cn } from '@/lib/utils'

interface SystemMessageProps {
  message: UiMessage
}

// Extract text content from UiMessage content blocks
function getTextContent(message: UiMessage): string {
  return message.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text ?? '')
    .join('\n')
}

export function SystemMessageComponent({ message }: SystemMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = (message.messageType as string) === 'transcript_only'
  const isSummary = (message.messageType as string) === 'compact_summary'
  const textContent = getTextContent(message)

  // For hidden/summary messages, show collapsed with expand option
  if (isHidden && !isExpanded) {
    return (
      <div className="flex items-center gap-2 py-1 px-2 text-xs text-muted-foreground">
        <button
          type="button"
          onClick={() => setIsExpanded(true)}
          className="flex items-center gap-1 hover:text-foreground"
        >
          <ChevronRight className="h-3 w-3" />
          <Settings className="h-3 w-3" />
          <span>System message (click to expand)</span>
        </button>
      </div>
    )
  }

  return (
    <div className={cn('py-2', isSummary && 'opacity-70')}>
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground mb-1"
      >
        {isExpanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        <Settings className="h-3 w-3" />
        <span>System message</span>
      </button>

      {isExpanded && (
        <div className="ml-5 border-l-2 border-muted pl-3">
          <Response>{textContent}</Response>
        </div>
      )}
    </div>
  )
}

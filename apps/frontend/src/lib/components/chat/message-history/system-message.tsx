import type { SystemMessage } from '@moatless/types'
import { ChevronDown, ChevronRight, Settings } from 'lucide-react'
import { useState } from 'react'
import { Response } from '@/lib/components/ai-elements/response'
import { cn } from '@/lib/utils'

interface SystemMessageProps {
  message: SystemMessage
}

export function SystemMessageComponent({ message }: SystemMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = message.isVisibleInTranscriptOnly
  const isSummary = message.isCompactSummary

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
          <Response>{message.content}</Response>
        </div>
      )}
    </div>
  )
}

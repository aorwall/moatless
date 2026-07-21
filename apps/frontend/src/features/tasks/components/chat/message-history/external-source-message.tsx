import type { UiMessage } from '@moatless/api-client'
import { Bot, ChevronDown, ChevronUp } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import { getAdapterIcon, getAdapterLabel } from './external-source-meta'

interface ExternalSourceMessageProps {
  message: UiMessage
  variant?: 'external-source' | 'task'
}

const COLLAPSED_LINES = 4
const COLLAPSED_CHAR_THRESHOLD = 400

function getTextContent(message: UiMessage): string {
  return message.content.flatMap((c) => (c.type === 'text' ? [c.text ?? ''] : [])).join('\n')
}

function CollapsibleBody({ textContent }: { textContent: string }) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isLong =
    textContent.split('\n').length > COLLAPSED_LINES ||
    textContent.length > COLLAPSED_CHAR_THRESHOLD

  return (
    <>
      <pre
        className={cn(
          'px-3 py-2 text-sm font-mono whitespace-pre-wrap break-words overflow-x-auto text-foreground/90',
          isLong && !isExpanded && 'line-clamp-4',
        )}
      >
        {textContent}
      </pre>
      {isLong && (
        <button
          type="button"
          onClick={() => setIsExpanded(!isExpanded)}
          className="w-full flex items-center gap-1 px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
        >
          {isExpanded ? (
            <>
              <ChevronUp className="h-3 w-3 flex-shrink-0" />
              Show less
            </>
          ) : (
            <>
              <ChevronDown className="h-3 w-3 flex-shrink-0" />
              Show more
            </>
          )}
        </button>
      )}
    </>
  )
}

export function ExternalSourceMessageComponent({
  message,
  variant = 'external-source',
}: ExternalSourceMessageProps) {
  const textContent = getTextContent(message)

  if (variant === 'task') {
    return (
      <div className="w-full rounded-lg border-l-4 border-violet-500/50 bg-violet-500/5 dark:bg-violet-500/10 overflow-hidden">
        <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground border-b border-violet-500/10">
          <span className="text-violet-500/70">
            <Bot className="h-3.5 w-3.5" />
          </span>
          <span className="font-medium">from task</span>
          {message.sourceTaskId && (
            <span className="ml-auto font-mono text-[10px] text-muted-foreground/60">
              {message.sourceTaskId}
            </span>
          )}
        </div>
        <CollapsibleBody textContent={textContent} />
      </div>
    )
  }

  const source = message.externalSource
  const adapterKind = source?.adapterKind ?? 'integration'
  const sourceName = source?.sourceName
  const externalId = source?.externalId
  const externalLabel = source?.externalLabel ?? externalId

  return (
    <div className="w-full rounded-lg border-l-4 border-blue-500/50 bg-blue-500/5 dark:bg-blue-500/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground border-b border-blue-500/10">
        <span className="text-blue-500/70">{getAdapterIcon(adapterKind)}</span>
        <span className="font-medium">via {getAdapterLabel(adapterKind)}</span>
        {sourceName && <span className="text-muted-foreground/70">{sourceName}</span>}
        {externalLabel && (
          <span
            className="ml-auto font-mono text-[10px] text-muted-foreground/60"
            title={externalId ?? externalLabel}
          >
            {externalLabel}
          </span>
        )}
      </div>
      <CollapsibleBody textContent={textContent} />
    </div>
  )
}

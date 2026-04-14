import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import {
  CheckCircle,
  ChevronDown,
  ChevronRight,
  FileText,
  Loader2,
  Users,
  XCircle,
} from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { cn } from '@/lib/utils'

interface SubagentToolCallProps {
  content: UiContentBlock
  toolResult?: UiToolResult
  subagentMessages?: UiMessage[]
  expanded: boolean
  onToggle: () => void
}

interface NestedToolCallItemProps {
  message: UiMessage
}

function NestedToolCallItem({ message }: NestedToolCallItemProps) {
  const [expanded, setExpanded] = useState(false)
  const toolCalls = message.content.filter((c) => c.type === 'tool_use')
  const textContent = message.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')

  if (toolCalls.length === 0 && !textContent) {
    return null
  }

  return (
    <div className="space-y-1">
      {textContent && (
        <div className="text-muted-foreground text-xs italic truncate">{textContent}</div>
      )}
      {toolCalls.map((toolCall) => {
        const input = toolCall.toolInput as Record<string, unknown> | undefined
        const firstParam = input ? Object.entries(input)[0] : undefined
        const displayValue =
          firstParam && typeof firstParam[1] === 'string'
            ? firstParam[1].slice(0, 40)
            : firstParam
              ? JSON.stringify(firstParam[1]).slice(0, 40)
              : ''

        // Find matching tool result
        const result = message.toolResults?.find((r) => r.toolUseId === toolCall.toolUseId)
        const hasResult = !!result
        const isError = result?.isError

        return (
          <div key={toolCall.toolUseId} className="flex items-start gap-1.5">
            <div
              className={cn(
                'w-1 h-1 rounded-full mt-1.5 flex-shrink-0',
                isError ? 'bg-destructive' : hasResult ? 'bg-green-500' : 'bg-muted-foreground',
              )}
            />
            <div className="flex-1 min-w-0">
              <button
                type="button"
                onClick={() => setExpanded(!expanded)}
                className="flex items-center gap-1 text-left w-full hover:text-foreground transition-colors"
              >
                {hasResult && (
                  <span className="flex-shrink-0">
                    {expanded ? (
                      <ChevronDown className="h-2.5 w-2.5" />
                    ) : (
                      <ChevronRight className="h-2.5 w-2.5" />
                    )}
                  </span>
                )}
                <span className="font-medium flex-shrink-0">{toolCall.toolName}</span>
                <span className="text-muted-foreground truncate min-w-0 text-xs">
                  {displayValue}
                </span>
              </button>
              {expanded && result?.content && (
                <div className="mt-1 bg-muted/30 p-1.5 rounded text-xs text-muted-foreground whitespace-pre-wrap break-all max-h-32 overflow-y-auto">
                  {result.content}
                </div>
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}

export function SubagentToolCall({
  content,
  toolResult,
  subagentMessages,
  expanded,
  onToggle,
}: SubagentToolCallProps) {
  const input = content.toolInput as { prompt?: string; description?: string } | undefined
  const description = input?.description || input?.prompt?.slice(0, 60) || 'Subagent task'

  // Determine status
  const isComplete = !!toolResult
  const isError = toolResult?.isError
  const isRunning = !isComplete && subagentMessages && subagentMessages.length > 0

  // Count tool calls in subagent messages
  const toolCallCount =
    subagentMessages?.reduce((count, msg) => {
      return count + msg.content.filter((c) => c.type === 'tool_use').length
    }, 0) ?? 0

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div
          className={cn(
            'w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0',
            isError ? 'bg-destructive' : isComplete ? 'bg-green-500' : 'bg-purple-500',
          )}
        />
        <div className="flex-1 min-w-0">
          {/* Header */}
          <button
            type="button"
            onClick={onToggle}
            className="flex items-center gap-1.5 min-w-0 w-full text-left hover:text-foreground transition-colors"
          >
            <span className="flex-shrink-0">
              {expanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
            </span>
            <Users className="h-3 w-3 text-purple-500 flex-shrink-0" />
            <span className="font-medium flex-shrink-0">Agent</span>
            <span className="text-muted-foreground truncate min-w-0">{description}</span>

            {/* Status indicators */}
            <div className="flex items-center gap-1.5 ml-auto flex-shrink-0">
              {toolCallCount > 0 && (
                <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
                  {toolCallCount} tool{toolCallCount !== 1 ? 's' : ''}
                </Badge>
              )}
              {isRunning && <Loader2 className="h-3 w-3 text-purple-500 animate-spin" />}
              {isComplete && !isError && <CheckCircle className="h-3 w-3 text-green-500" />}
              {isError && <XCircle className="h-3 w-3 text-destructive" />}
            </div>
          </button>

          {/* Expanded content - nested tool calls */}
          {expanded && subagentMessages && subagentMessages.length > 0 && (
            <div className="mt-2 ml-2 border-l-2 border-purple-500/30 pl-3 space-y-2">
              {subagentMessages.map((msg) => (
                <NestedToolCallItem key={msg.uuid} message={msg} />
              ))}
            </div>
          )}

          {/* Tool result (agent's final response) */}
          {expanded && toolResult && (
            <div className="mt-2 ml-2 border-l-2 border-purple-500/30 pl-3">
              <div className="flex items-center gap-1 text-muted-foreground mb-1">
                <FileText className="h-3 w-3" />
                <span>Result</span>
              </div>
              <div
                className={cn(
                  'bg-muted/30 p-2 rounded text-xs whitespace-pre-wrap break-words max-h-48 overflow-y-auto',
                  isError && 'bg-destructive/10 text-destructive',
                )}
              >
                {toolResult.content || '(no content)'}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import { Terminal, ChevronRight, ChevronDown } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { useState } from 'react'

interface AssistantMessageProps {
  message: AssistantMessage
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

interface ToolCallProps {
  content: AssistantMessageContent
  expanded: boolean
  onToggle: () => void
}

function ToolCallComponent({ content, expanded, onToggle }: ToolCallProps) {
  const hasResult = !!content.tool_result
  const resultLines = content.tool_result?.content?.split('\n') || []
  const shouldTruncate = resultLines.length > 3
  const displayLines = expanded ? resultLines : resultLines.slice(0, 3)
  const hiddenLineCount = resultLines.length - 3
  
  // Format tool input for display
  const formatToolInput = () => {
    if (!content.tool_input) return ''
    if (typeof content.tool_input === 'string') return content.tool_input
    
    const entries = Object.entries(content.tool_input)
    if (entries.length === 0) return ''
    
    // Show only first parameter in compact view
    const [_key, value] = entries[0]
    const displayValue = typeof value === 'string' ? value : JSON.stringify(value)
    const truncatedValue = displayValue.length > 50 ? `${displayValue.substring(0, 50)}...` : displayValue
    
    return entries.length > 1 
      ? `${truncatedValue} +${entries.length - 1} more`
      : truncatedValue
  }

  return (
    <div className="text-xs font-mono">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-blue-500 rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1">
            <span className="font-medium">{content.tool_name}</span>
            {content.tool_input && (
              <span className="text-muted-foreground truncate">
                ({formatToolInput()})
              </span>
            )}
          </div>

          {hasResult && (
            <div className="mt-1 ml-2 pl-2 border-l-2 border-border/50">
              <div className="bg-muted/30 rounded p-1.5 text-muted-foreground">
                {displayLines.map((line, index) => (
                  <div key={`${content.tool_use_id}-${index}`} className="whitespace-pre-wrap break-all">
                    {line || ' '}
                  </div>
                ))}

                {shouldTruncate && (
                  <Button
                    variant="link"
                    size="sm"
                    onClick={onToggle}
                    className="h-auto p-0 text-xs text-muted-foreground hover:text-foreground"
                  >
                    {expanded ? '← collapse' : `… +${hiddenLineCount} lines →`}
                  </Button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

interface ToolCallSummaryProps {
  toolCalls: AssistantMessageContent[]
  expanded: boolean
  onToggle: () => void
}

function ToolCallSummary({ toolCalls, expanded, onToggle }: ToolCallSummaryProps) {
  // Group tool calls by name
  const toolCallCounts = toolCalls.reduce((acc, tool) => {
    const name = tool.tool_name || 'Unknown'
    acc[name] = (acc[name] || 0) + 1
    return acc
  }, {} as Record<string, number>)

  const totalCalls = toolCalls.length
  const toolNames = Object.entries(toolCallCounts)
    .map(([name, count]) => count > 1 ? `${name} (${count})` : name)
    .join(', ')

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={onToggle}
      className="w-full justify-start p-2 h-auto font-mono text-xs hover:bg-muted/50"
    >
      <div className="flex items-center gap-2 w-full">
        {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        <Terminal className="h-3 w-3 text-muted-foreground" />
        <span className="text-muted-foreground">
          {totalCalls} tool {totalCalls === 1 ? 'call' : 'calls'}: {toolNames}
        </span>
      </div>
    </Button>
  )
}

export function AssistantMessageComponent({
  message,
  expandedToolCalls,
  onToggleToolCall,
}: AssistantMessageProps) {
  const [toolCallsExpanded, setToolCallsExpanded] = useState(false)
  const isStringContent = typeof message.content === 'string'
  const contentArray = isStringContent ? [] : (message.content as AssistantMessageContent[])

  // Separate text content from tool calls
  const textContent = isStringContent
    ? (message.content as string)
    : contentArray
        .filter((c) => c.type === 'text')
        .map((c) => c.text)
        .join('\n')

  const toolCalls = isStringContent ? [] : contentArray.filter((c) => c.type === 'tool_use')

  return (
    <div className="space-y-2">
      {/* Text content */}
      {textContent && (
        <div className="text-sm whitespace-pre-wrap break-words">{textContent}</div>
      )}

      {/* Tool calls */}
      {toolCalls.length > 0 && (
        <div className="border border-border/50 rounded-lg overflow-hidden">
          <ToolCallSummary
            toolCalls={toolCalls}
            expanded={toolCallsExpanded}
            onToggle={() => setToolCallsExpanded(!toolCallsExpanded)}
          />
          
          {toolCallsExpanded && (
            <div className="p-2 space-y-2 border-t border-border/50">
              {toolCalls.map((toolCall) => (
                <ToolCallComponent
                  key={toolCall.tool_use_id}
                  content={toolCall}
                  expanded={expandedToolCalls.has(toolCall.tool_use_id || '')}
                  onToggle={() => onToggleToolCall(toolCall.tool_use_id || '')}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

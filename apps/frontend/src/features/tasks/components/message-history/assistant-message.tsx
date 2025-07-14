import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import { Bot, Terminal } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

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

  return (
    <div className="border border-border rounded-lg p-3 bg-background/50">
      <div className="flex items-center gap-2 mb-2">
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 bg-blue-500 rounded-full" />
          <Terminal className="h-4 w-4 text-muted-foreground" />
          <span className="font-mono text-sm font-medium">
            {content.tool_name}
            {content.tool_input && (
              <span className="text-muted-foreground">
                (
                {typeof content.tool_input === 'string'
                  ? content.tool_input
                  : Object.keys(content.tool_input).length > 0
                    ? Object.entries(content.tool_input)
                        .map(([, value]) =>
                          typeof value === 'string' ? value : JSON.stringify(value),
                        )
                        .join(', ')
                    : ''}
                )
              </span>
            )}
          </span>
        </div>
      </div>

      {hasResult && (
        <div className="ml-6">
          <div className="flex items-start gap-1">
            <div className="text-muted-foreground text-sm mt-1">⎿</div>
            <div className="flex-1">
              <div className="font-mono text-sm bg-muted/50 rounded p-2 border">
                {displayLines.map((line, index) => (
                  <div key={index} className="whitespace-pre-wrap break-all">
                    {line}
                  </div>
                ))}

                {shouldTruncate && !expanded && (
                  <div className="flex items-center gap-2 mt-2 pt-2 border-t border-border/50">
                    <span className="text-muted-foreground text-xs">
                      … +{hiddenLineCount} lines
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={onToggle}
                      className="h-6 px-2 py-0 text-xs text-muted-foreground hover:text-foreground"
                    >
                      (ctrl+r to expand)
                    </Button>
                  </div>
                )}

                {shouldTruncate && expanded && (
                  <div className="flex items-center gap-2 mt-2 pt-2 border-t border-border/50">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={onToggle}
                      className="h-6 px-2 py-0 text-xs text-muted-foreground hover:text-foreground"
                    >
                      (collapse)
                    </Button>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export function AssistantMessageComponent({
  message,
  expandedToolCalls,
  onToggleToolCall,
}: AssistantMessageProps) {
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
    <div className="flex justify-start">
      <div className="bg-secondary rounded-lg p-3 max-w-[80%] shadow-sm">
        <div className="flex items-center gap-2 mb-2">
          <Bot className="h-4 w-4 text-blue-500" />
          <span className="text-sm font-medium">Assistant</span>
          <span className="text-xs text-muted-foreground">
            {new Date(message.timestamp).toLocaleTimeString()}
          </span>
        </div>

        {/* Text content */}
        {textContent && (
          <div className="text-sm whitespace-pre-wrap break-words mb-3">{textContent}</div>
        )}

        {/* Tool calls */}
        {toolCalls.length > 0 && (
          <div className="space-y-2">
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
    </div>
  )
}

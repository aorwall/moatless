import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Button } from '@/lib/components/ui/button'
import { type Todo, TodoList } from './tool-calls'

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
  // Special handling for TodoWrite tool
  if (content.tool_name === 'TodoWrite' && content.tool_input) {
    const toolInput = content.tool_input as { todos?: Todo[] }
    if (toolInput.todos && Array.isArray(toolInput.todos)) {
      return (
        <div className="text-xs font-mono w-full max-w-full">
          <div className="flex items-start gap-2">
            <div className="w-1.5 h-1.5 bg-blue-500 rounded-full mt-1 flex-shrink-0" />
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-1 mb-2">
                <span className="font-medium">{content.tool_name}</span>
                <span className="text-muted-foreground">({toolInput.todos.length} items)</span>
              </div>
              <TodoList todos={toolInput.todos} />
            </div>
          </div>
        </div>
      )
    }
  }

  const hasResult = !!content.tool_result
  const resultLines = content.tool_result?.content?.split('\n') || []
  const shouldTruncate = resultLines.length > 3
  const displayLines = expanded ? resultLines : resultLines.slice(0, 2)
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
    const truncatedValue =
      displayValue.length > 50 ? `${displayValue.substring(0, 50)}...` : displayValue

    return entries.length > 1 ? `${truncatedValue} +${entries.length - 1} more` : truncatedValue
  }

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-blue-500 rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1 min-w-0">
            <span className="font-medium flex-shrink-0">{content.tool_name}</span>
            {content.tool_input && (
              <span className="text-muted-foreground truncate min-w-0">({formatToolInput()})</span>
            )}
          </div>

          {hasResult && (
            <div className="mt-1">
              <div className="bg-muted/30 p-1 text-muted-foreground">
                {displayLines.map((line, index) => (
                  <div
                    key={`${content.tool_use_id}-${index}`}
                    className="whitespace-pre-wrap break-all"
                  >
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
    <div className="space-y-2 w-full max-w-full">
      {/* Text content */}
      {textContent && (
        <div className="text-sm prose prose-sm dark:prose-invert prose-max-w-none w-full max-w-full break-words">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{textContent}</ReactMarkdown>
        </div>
      )}

      {/* Tool calls */}
      {toolCalls.length > 0 && (
        <div className="space-y-2 w-full max-w-full">
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
  )
}

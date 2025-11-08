import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import { type ImageContentBlock, isImageContentBlock } from '@moatless/types'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { Response } from '@/components/ai-elements/response'
import { ImageThumbnail } from './image-thumbnail'
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

/**
 * Parse tool result content to extract image data if present
 * Tool results may contain JSON with image blocks like:
 * {"type":"image","source":{"type":"base64","data":"...","media_type":"image/png"}}
 */
function parseImageFromToolResult(content: string | undefined): ImageContentBlock | null {
  if (!content || typeof content !== 'string') {
    return null
  }

  try {
    const parsed = JSON.parse(content)
    if (isImageContentBlock(parsed)) {
      return parsed
    }
  } catch {
    // Not JSON or invalid format, ignore
  }

  return null
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Component handles multiple tool types and states
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
  const imageData = parseImageFromToolResult(content.tool_result?.content)
  const resultLines = content.tool_result?.content?.split('\n') || []
  const lineCount = resultLines.length

  // Format tool input for display
  const formatToolInput = () => {
    if (!content.tool_input) {
      return ''
    }
    if (typeof content.tool_input === 'string') {
      return content.tool_input
    }

    const entries = Object.entries(content.tool_input)
    if (entries.length === 0) {
      return ''
    }

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
          {/* Clickable header */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
          {/* biome-ignore lint/a11y/useKeyWithClickEvents: Keyboard handler is added conditionally with onClick */}
          <div
            role={hasResult && lineCount > 0 ? 'button' : undefined}
            tabIndex={hasResult && lineCount > 0 ? 0 : undefined}
            onClick={hasResult && lineCount > 0 ? onToggle : undefined}
            onKeyDown={
              hasResult && lineCount > 0
                ? (e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault()
                      onToggle()
                    }
                  }
                : undefined
            }
            className={`flex items-center gap-1 min-w-0 ${
              hasResult && lineCount > 0
                ? 'cursor-pointer hover:text-foreground transition-colors'
                : ''
            }`}
          >
            {hasResult && lineCount > 0 && (
              <span className="flex-shrink-0">
                {expanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
              </span>
            )}
            <span className="font-medium flex-shrink-0">{content.tool_name}</span>
            {content.tool_input && (
              <span className="text-muted-foreground truncate min-w-0">({formatToolInput()})</span>
            )}
            {hasResult && lineCount > 0 && (
              <span className="text-muted-foreground flex-shrink-0">
                {lineCount} {lineCount === 1 ? 'line' : 'lines'}
              </span>
            )}
          </div>

          {/* Result - only shown when expanded */}
          {hasResult && expanded && (
            <div className="mt-1">
              {imageData ? (
                <div className="p-1">
                  <ImageThumbnail
                    data={imageData.source.data}
                    mediaType={imageData.source.media_type}
                    alt={`${content.tool_name} result`}
                    size="expanded"
                  />
                </div>
              ) : (
                lineCount > 0 && (
                  <div className="bg-muted/30 p-1 text-muted-foreground">
                    {resultLines.map((line, index) => (
                      <div
                        key={`${content.tool_use_id}-${index}`}
                        className="whitespace-pre-wrap break-all"
                      >
                        {line || ' '}
                      </div>
                    ))}
                  </div>
                )
              )}
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
  // Content is always an array now (backend ensures this)
  const contentArray = Array.isArray(message.content) ? message.content : []

  // Separate text content from tool calls
  const textContent = contentArray
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')

  const toolCalls = contentArray.filter((c) => c.type === 'tool_use')

  return (
    <div className="space-y-2 w-full max-w-full">
      {/* Text content */}
      {textContent && (
        <div className="text-sm w-full max-w-full">
          <Response>{textContent}</Response>
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

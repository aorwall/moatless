import type {
  AllowedPrompt,
  AssistantMessage,
  AssistantMessageContent,
  Question,
} from '@moatless/types'
import { type ImageContentBlock, isImageContentBlock } from '@moatless/types'
import {
  CheckCircle,
  ChevronDown,
  ChevronRight,
  Eye,
  FileText,
  HelpCircle,
  XCircle,
} from 'lucide-react'
import { useState } from 'react'
import { Response } from '@/lib/components/ai-elements/response'
import { cn } from '@/lib/utils'
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
  hasPendingUserInput?: boolean
}

/**
 * Parse tool result content to extract image data if present
 * Tool results may contain JSON with image blocks like:
 * {"type":"image","source":{"type":"base64","data":"...","media_type":"image/png"}}
 */
function parseImageFromToolResult(content: string | undefined): ImageContentBlock | null {
  if (!content) {
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
function ToolCallComponent({ content, expanded, onToggle, hasPendingUserInput }: ToolCallProps) {
  // Special handling for AskUserQuestion tool
  if (content.tool_name === 'AskUserQuestion' && content.tool_input) {
    const toolInput = content.tool_input as { questions?: Question[] }
    if (toolInput.questions && Array.isArray(toolInput.questions)) {
      // If pending (will be rendered in input area), skip rendering here
      if (hasPendingUserInput && !content.tool_result) {
        return null
      }

      // Parse answers from tool result - try JSON first, fallback to displaying content
      let answers: Record<string, string> | undefined
      let answerText: string | undefined

      if (content.tool_result?.content) {
        try {
          const result = JSON.parse(content.tool_result.content)
          if (result.answers && typeof result.answers === 'object') {
            answers = result.answers
          }
        } catch {
          // Not JSON - use raw text as fallback
          answerText = content.tool_result.content
        }
      }

      const hasResponse = !!answers || !!answerText
      const isExpandable = hasResponse

      // Show as completed question summary
      const questionCount = toolInput.questions.length
      const firstQuestion = toolInput.questions[0]?.question || ''
      const truncatedQuestion =
        firstQuestion.length > 60 ? `${firstQuestion.slice(0, 60)}...` : firstQuestion

      return (
        <div className="text-xs font-mono w-full max-w-full">
          <div className="flex items-start gap-2">
            <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
            <div className="flex-1 min-w-0">
              {/* Header */}
              {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
              <div
                role={isExpandable ? 'button' : undefined}
                tabIndex={isExpandable ? 0 : undefined}
                onClick={isExpandable ? onToggle : undefined}
                onKeyDown={
                  isExpandable
                    ? (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          onToggle()
                        }
                      }
                    : undefined
                }
                className={cn(
                  'flex items-center gap-1.5 min-w-0',
                  isExpandable && 'cursor-pointer hover:text-foreground transition-colors',
                )}
              >
                {isExpandable && (
                  <span className="flex-shrink-0">
                    {expanded ? (
                      <ChevronDown className="h-3 w-3" />
                    ) : (
                      <ChevronRight className="h-3 w-3" />
                    )}
                  </span>
                )}
                <HelpCircle className="h-3 w-3 text-primary flex-shrink-0" />
                <span className="font-medium flex-shrink-0">
                  {questionCount === 1 ? 'Asked question' : `Asked ${questionCount} questions`}
                </span>
                <span className="text-muted-foreground truncate min-w-0">{truncatedQuestion}</span>
                {hasResponse && (
                  <CheckCircle className="h-3 w-3 text-green-500 flex-shrink-0 ml-auto" />
                )}
              </div>

              {/* Expanded answers */}
              {isExpandable && expanded && (
                <div className="mt-2 space-y-2 bg-muted/30 p-2 rounded">
                  {answers
                    ? toolInput.questions.map((q) => (
                        <div key={q.question} className="space-y-0.5">
                          <div className="text-muted-foreground">{q.question}</div>
                          <div className="font-medium text-green-600 dark:text-green-400">
                            → {answers[q.question] || '(no answer)'}
                          </div>
                        </div>
                      ))
                    : answerText && <div className="text-sm">{answerText}</div>}
                </div>
              )}
            </div>
          </div>
        </div>
      )
    }
  }

  // Special handling for ExitPlanMode tool
  if (content.tool_name === 'ExitPlanMode') {
    // If pending (will be rendered in input area), skip rendering here
    if (hasPendingUserInput && !content.tool_result) {
      return null
    }

    const toolInput = content.tool_input as
      | { allowedPrompts?: AllowedPrompt[]; plan?: string }
      | undefined
    const hasPlan = !!toolInput?.plan
    const isExpandable = hasPlan
    const isResolved = !!content.tool_result
    const wasApproved = content.tool_result && !content.tool_result.is_error

    return (
      <div className="text-xs font-mono w-full max-w-full">
        <div className="flex items-start gap-2">
          <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
          <div className="flex-1 min-w-0">
            {/* Header */}
            {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
            <div
              role={isExpandable ? 'button' : undefined}
              tabIndex={isExpandable ? 0 : undefined}
              onClick={isExpandable ? onToggle : undefined}
              onKeyDown={
                isExpandable
                  ? (e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        onToggle()
                      }
                    }
                  : undefined
              }
              className={cn(
                'flex items-center gap-1.5 min-w-0',
                isExpandable && 'cursor-pointer hover:text-foreground transition-colors',
              )}
            >
              {isExpandable && (
                <span className="flex-shrink-0">
                  {expanded ? (
                    <ChevronDown className="h-3 w-3" />
                  ) : (
                    <ChevronRight className="h-3 w-3" />
                  )}
                </span>
              )}
              <FileText className="h-3 w-3 text-primary flex-shrink-0" />
              <span className="font-medium flex-shrink-0">Plan approval</span>
              {isResolved ? (
                wasApproved ? (
                  <>
                    <CheckCircle className="h-3 w-3 text-green-500 flex-shrink-0" />
                    <span className="text-green-600 dark:text-green-400">Approved</span>
                  </>
                ) : (
                  <>
                    <XCircle className="h-3 w-3 text-red-500 flex-shrink-0" />
                    <span className="text-red-600 dark:text-red-400">Rejected</span>
                  </>
                )
              ) : (
                <span className="text-muted-foreground">Pending</span>
              )}
            </div>

            {/* Expanded plan content */}
            {isExpandable && expanded && (
              <div className="mt-2 bg-muted/30 p-3 rounded max-h-96 overflow-y-auto">
                <Response>{toolInput?.plan}</Response>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }

  // Special handling for TodoWrite tool
  if (content.tool_name === 'TodoWrite' && content.tool_input) {
    const toolInput = content.tool_input as { todos?: Todo[] }
    if (toolInput.todos && Array.isArray(toolInput.todos)) {
      return (
        <div className="text-xs font-mono w-full max-w-full">
          <div className="flex items-start gap-2">
            <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
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
        <div
          className={cn(
            'w-1.5 h-1.5 rounded-full mt-1 flex-shrink-0',
            content.tool_result?.is_error ? 'bg-destructive' : 'bg-primary',
          )}
        />
        <div className="flex-1 min-w-0">
          {/* Clickable header */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
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
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = message.isVisibleInTranscriptOnly
  const isSummary = message.isCompactSummary

  // Handle both string content (legacy/stories) and array content (backend)
  const contentArray = Array.isArray(message.content)
    ? message.content
    : typeof message.content === 'string'
      ? [{ type: 'text' as const, text: message.content }]
      : []

  // Separate text content from tool calls
  const textContent = contentArray
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')

  const toolCalls = contentArray.filter((c) => c.type === 'tool_use')

  // For hidden messages, show collapsed view by default
  if (isHidden) {
    const previewText =
      textContent || (toolCalls.length > 0 ? `${toolCalls.length} tool calls` : '')

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
              <span className="font-medium">Summary Response</span>
            </>
          ) : (
            <>
              <Eye className="h-3 w-3 flex-shrink-0" />
              <span className="font-medium">Hidden Response</span>
            </>
          )}
          <span className="text-muted-foreground/70 truncate">
            {previewText.slice(0, 50)}
            {previewText.length > 50 ? '...' : ''}
          </span>
        </button>

        {isExpanded && (
          <div className="p-3 border-t border-dashed border-muted-foreground/20">
            <div className="space-y-2 opacity-80">
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
                      hasPendingUserInput={message.hasPendingUserInput}
                    />
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    )
  }

  // Normal message display
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
              hasPendingUserInput={message.hasPendingUserInput}
            />
          ))}
        </div>
      )}
    </div>
  )
}

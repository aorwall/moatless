import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import type { AllowedPrompt, Question } from '@moatless/types'
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
  message: UiMessage
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

interface ToolCallProps {
  content: UiContentBlock
  toolResult?: UiToolResult
  expanded: boolean
  onToggle: () => void
  hasPendingUserInput?: boolean | null
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
function ToolCallComponent({
  content,
  toolResult,
  expanded,
  onToggle,
  hasPendingUserInput,
}: ToolCallProps) {
  // Special handling for AskUserQuestion tool
  if (content.toolName === 'AskUserQuestion' && content.toolInput) {
    const toolInput = content.toolInput as { questions?: Question[] }
    if (toolInput.questions && Array.isArray(toolInput.questions)) {
      // If pending (will be rendered in input area), skip rendering here
      if (hasPendingUserInput && !toolResult) {
        return null
      }

      // Parse answers from tool result - try JSON first, fallback to displaying content
      let answers: Record<string, string> | undefined
      let answerText: string | undefined

      if (toolResult?.content) {
        try {
          const result = JSON.parse(toolResult.content)
          if (result.answers && typeof result.answers === 'object') {
            answers = result.answers
          }
        } catch {
          // Not JSON - use raw text as fallback
          answerText = toolResult.content
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
  if (content.toolName === 'ExitPlanMode') {
    // If pending (will be rendered in input area), skip rendering here
    if (hasPendingUserInput && !toolResult) {
      return null
    }

    const toolInput = content.toolInput as
      | { allowedPrompts?: AllowedPrompt[]; plan?: string }
      | undefined
    const hasPlan = !!toolInput?.plan
    const isExpandable = hasPlan
    const isResolved = !!toolResult
    const wasApproved = toolResult && !toolResult.isError

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
  if (content.toolName === 'TodoWrite' && content.toolInput) {
    const toolInput = content.toolInput as { todos?: Todo[] }
    if (toolInput.todos && Array.isArray(toolInput.todos)) {
      return (
        <div className="text-xs font-mono w-full max-w-full">
          <div className="flex items-start gap-2">
            <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-1 mb-2">
                <span className="font-medium">{content.toolName}</span>
                <span className="text-muted-foreground">({toolInput.todos.length} items)</span>
              </div>
              <TodoList todos={toolInput.todos} />
            </div>
          </div>
        </div>
      )
    }
  }

  const hasResult = !!toolResult
  const imageData = parseImageFromToolResult(toolResult?.content ?? undefined)
  const resultLines = toolResult?.content?.split('\n') || []
  const lineCount = resultLines.length

  // Format tool input for display
  const formatToolInput = () => {
    if (!content.toolInput) {
      return ''
    }

    const entries = Object.entries(content.toolInput)
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
            toolResult?.isError ? 'bg-destructive' : 'bg-primary',
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
            <span className="font-medium flex-shrink-0">{content.toolName}</span>
            {content.toolInput && (
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
                    alt={`${content.toolName} result`}
                    size="expanded"
                  />
                </div>
              ) : (
                lineCount > 0 && (
                  <div className="bg-muted/30 p-1 text-muted-foreground">
                    {resultLines.map((line, index) => (
                      <div
                        key={`${content.toolUseId}-${index}`}
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

/**
 * Build a lookup map from toolUseId to toolResult for efficient retrieval
 */
function buildToolResultMap(message: UiMessage): Map<string, UiToolResult> {
  const toolResultMap = new Map<string, UiToolResult>()
  if (message.toolResults) {
    for (const tr of message.toolResults) {
      if (tr.toolUseId) {
        toolResultMap.set(tr.toolUseId, tr)
      }
    }
  }
  return toolResultMap
}

/**
 * Separate content array into text content and tool calls
 */
function parseMessageContent(contentArray: UiContentBlock[]): {
  textContent: string
  toolCalls: UiContentBlock[]
} {
  const textContent = contentArray
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')
  const toolCalls = contentArray.filter((c) => c.type === 'tool_use')
  return { textContent, toolCalls }
}

/**
 * Render tool calls list with result mapping
 */
function renderToolCalls(
  toolCalls: UiContentBlock[],
  toolResultMap: Map<string, UiToolResult>,
  expandedToolCalls: Set<string>,
  onToggleToolCall: (toolUseId: string) => void,
  hasPendingUserInput: boolean | null | undefined,
): React.ReactNode {
  if (toolCalls.length === 0) {
    return null
  }

  return (
    <div className="space-y-2 w-full max-w-full">
      {toolCalls.map((toolCall) => (
        <ToolCallComponent
          key={toolCall.toolUseId}
          content={toolCall}
          toolResult={toolCall.toolUseId ? toolResultMap.get(toolCall.toolUseId) : undefined}
          expanded={expandedToolCalls.has(toolCall.toolUseId || '')}
          onToggle={() => onToggleToolCall(toolCall.toolUseId || '')}
          hasPendingUserInput={hasPendingUserInput}
        />
      ))}
    </div>
  )
}

/**
 * Render a collapsed message view (for hidden/transcript-only messages)
 */
function renderCollapsedMessage(
  isExpanded: boolean,
  setIsExpanded: (value: boolean) => void,
  isSummary: boolean,
  previewText: string,
  textContent: string,
  toolCalls: UiContentBlock[],
  toolResultMap: Map<string, UiToolResult>,
  expandedToolCalls: Set<string>,
  onToggleToolCall: (toolUseId: string) => void,
  hasPendingUserInput: boolean | null | undefined,
): React.ReactNode {
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
            {textContent && (
              <div className="text-sm w-full max-w-full">
                <Response>{textContent}</Response>
              </div>
            )}

            {renderToolCalls(
              toolCalls,
              toolResultMap,
              expandedToolCalls,
              onToggleToolCall,
              hasPendingUserInput,
            )}
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
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = (message.messageType as string) === 'transcript_only'
  const isSummary = (message.messageType as string) === 'compact_summary'

  // Build a lookup map from toolUseId to toolResult
  const toolResultMap = buildToolResultMap(message)

  // Content is always UiContentBlock[]
  const { textContent, toolCalls } = parseMessageContent(message.content)

  // For hidden messages, show collapsed view by default
  if (isHidden) {
    const previewText =
      textContent || (toolCalls.length > 0 ? `${toolCalls.length} tool calls` : '')

    return renderCollapsedMessage(
      isExpanded,
      setIsExpanded,
      isSummary,
      previewText,
      textContent,
      toolCalls,
      toolResultMap,
      expandedToolCalls,
      onToggleToolCall,
      message.hasPendingUserInput,
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
      {renderToolCalls(
        toolCalls,
        toolResultMap,
        expandedToolCalls,
        onToggleToolCall,
        message.hasPendingUserInput,
      )}
    </div>
  )
}

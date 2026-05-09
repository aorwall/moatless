import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import {
  AlertTriangle,
  Brain,
  CheckCircle,
  ChevronDown,
  ChevronRight,
  Eye,
  FileText,
  HelpCircle,
  Loader2,
  MessageSquare,
  MessageSquarePlus,
  XCircle,
} from 'lucide-react'
import { useState } from 'react'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import { useTaskFeedbackIndex } from '@/features/feedback/hooks/use-task-feedback'
import { useTaskToolCall } from '@/features/tasks/hooks/use-tasks'
import type { UiMessageWithSubagents } from '@/features/tasks/utils/group-messages'
import { Response } from '@/lib/components/ai-elements/response'
import { type ImageContentBlock, isImageContentBlock } from '@/lib/types/content-guards'
import type { AllowedPrompt, Question } from '@/lib/types/user-input'
import { cn } from '@/lib/utils'
import { getProviderIcon, getProviderLabel } from './channel-meta'
import { ImageThumbnail } from './image-thumbnail'
import { CodexToolCall, SubagentToolCall, type Todo, TodoList } from './tool-calls'

interface AssistantMessageProps {
  taskId?: string
  message: UiMessageWithSubagents
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

function ChannelChip({ channel }: { channel: NonNullable<UiMessage['channel']> }) {
  if (!channel.externalLabel) {
    return null
  }
  return (
    <div className="flex items-center gap-1.5 mb-1 text-xs text-muted-foreground">
      <span className="text-blue-500/70">{getProviderIcon(channel.provider)}</span>
      <span className="font-medium">via {getProviderLabel(channel.provider)}</span>
      {channel.sourceName && <span className="text-muted-foreground/70">{channel.sourceName}</span>}
      <span
        className="ml-auto font-mono text-[10px] text-muted-foreground/70"
        title={channel.externalId ?? channel.externalLabel}
      >
        {channel.externalLabel}
      </span>
    </div>
  )
}

interface ToolCallProps {
  taskId?: string
  /** UUID of the assistant message this tool call belongs to. */
  parentMessageUuid?: string
  content: UiContentBlock
  toolResult?: UiToolResult
  expanded: boolean
  onToggle: () => void
  hasPendingUserInput?: boolean | null
}

/**
 * Always-visible "has feedback" indicator shown next to an assistant text
 * bubble when at least one feedback row references its `messageUuid`.
 */
function AssistantMessageFeedbackIndicator({
  taskId,
  messageUuid,
}: {
  taskId: string
  messageUuid: string
}) {
  const index = useTaskFeedbackIndex(taskId)
  const count = index.byMessageUuid.get(messageUuid) ?? 0
  if (count === 0) {
    return null
  }
  return (
    <span
      className="mt-1 inline-flex items-center gap-1 text-[10px] text-muted-foreground"
      title={`${count} feedback note${count === 1 ? '' : 's'}`}
    >
      <MessageSquare className="h-3 w-3 fill-current" />
      {count}
    </span>
  )
}

/**
 * Hover-revealed feedback button + always-visible "has feedback" indicator
 * shown next to a tool call header. Returns `null` when no taskId is known
 * (sandbox preview mode etc.) so the chrome stays uncluttered.
 */
function ToolCallFeedbackChrome({
  taskId,
  parentMessageUuid,
  toolUseId,
}: {
  taskId?: string
  parentMessageUuid?: string
  toolUseId?: string
}) {
  const [open, setOpen] = useState(false)
  const index = useTaskFeedbackIndex(taskId)
  if (!(taskId && toolUseId)) {
    return null
  }
  const count = index.byToolUseId.get(toolUseId) ?? 0
  return (
    <span className="ml-auto flex items-center gap-1 flex-shrink-0">
      {count > 0 && (
        <span
          className="inline-flex items-center gap-0.5 text-[10px] text-muted-foreground"
          title={`${count} feedback note${count === 1 ? '' : 's'}`}
        >
          <MessageSquare className="h-3 w-3 fill-current" />
          {count}
        </span>
      )}
      <button
        type="button"
        aria-label="Give feedback on this tool call"
        title="Give feedback on this tool call"
        onClick={(e) => {
          e.stopPropagation()
          setOpen(true)
        }}
        className="opacity-0 group-hover:opacity-100 focus:opacity-100 rounded p-0.5 hover:bg-muted hover:text-foreground transition-opacity"
      >
        <MessageSquarePlus className="h-3 w-3" />
      </button>
      <FeedbackDialog
        open={open}
        onOpenChange={setOpen}
        taskId={taskId}
        messageUuid={parentMessageUuid}
        toolUseId={toolUseId}
        surface="tool_call"
      />
    </span>
  )
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
  taskId,
  parentMessageUuid,
  content,
  toolResult,
  expanded,
  onToggle,
  hasPendingUserInput,
}: ToolCallProps) {
  // Fetch full tool input + result on demand when this tool call is expanded.
  // The list endpoint returns a slim preview to keep page/SSE refresh payloads
  // small; the detail query stays cached across collapse/re-expand.
  const detailQuery = useTaskToolCall(taskId ?? '', content.toolUseId, expanded && !!taskId)
  const detail = detailQuery.data
  const detailIsLoading = detailQuery.isLoading

  // Codex tool calls use the `codex.*` namespace and are rendered via a
  // dedicated dispatch that knows how to pretty-print shell output and
  // file diffs. Unknown codex.* names fall through to the generic renderer.
  if (
    content.toolName === 'codex.shell' ||
    content.toolName === 'codex.edit' ||
    content.toolName === 'codex.todo_list'
  ) {
    return (
      <CodexToolCall
        content={content}
        toolResult={toolResult}
        expanded={expanded}
        onToggle={onToggle}
      />
    )
  }

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
  // Prefer the full content from the on-demand detail query when available,
  // falling back to the slim preview shipped on the list. This keeps the line
  // count + scroll height stable as the full data hydrates.
  const expandedContent = detail?.resultContent ?? toolResult?.content ?? undefined
  const imageData = parseImageFromToolResult(expandedContent)
  const resultLines = (toolResult?.content ?? '').split('\n')
  const lineCount = toolResult?.content ? resultLines.length : 0
  const expandedLines = expandedContent ? expandedContent.split('\n') : resultLines

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
    <div className="text-xs font-mono w-full max-w-full group">
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
            <ToolCallFeedbackChrome
              taskId={taskId}
              parentMessageUuid={parentMessageUuid}
              toolUseId={content.toolUseId ?? undefined}
            />
          </div>

          {/* Result - only shown when expanded */}
          {hasResult && expanded && (
            <div className="mt-1 space-y-1">
              {detail?.toolInput && (
                <pre className="bg-muted/20 p-1 text-muted-foreground whitespace-pre-wrap break-all">
                  {JSON.stringify(detail.toolInput, null, 2)}
                </pre>
              )}
              {detailIsLoading && !detail && (
                <div className="flex items-center gap-1 text-muted-foreground py-1">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  Loading…
                </div>
              )}
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
                expandedLines.length > 0 && (
                  <div className="bg-muted/30 p-1 text-muted-foreground">
                    {expandedLines.map((line, index) => (
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
export function buildToolResultMap(message: UiMessage): Map<string, UiToolResult> {
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
export function parseMessageContent(contentArray: UiContentBlock[]): {
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
export function renderToolCalls(
  taskId: string | undefined,
  toolCalls: UiContentBlock[],
  toolResultMap: Map<string, UiToolResult>,
  expandedToolCalls: Set<string>,
  onToggleToolCall: (toolUseId: string) => void,
  hasPendingUserInput: boolean | null | undefined,
  subagentMessages?: UiMessage[],
  parentMessageUuid?: string,
): React.ReactNode {
  if (toolCalls.length === 0) {
    return null
  }

  return (
    <div className="space-y-2 w-full max-w-full">
      {toolCalls.map((toolCall) => {
        // Render Agent tool calls with SubagentToolCall component
        if (toolCall.toolName === 'Agent' && toolCall.toolUseId) {
          return (
            <SubagentToolCall
              key={toolCall.toolUseId}
              content={toolCall}
              toolResult={toolResultMap.get(toolCall.toolUseId)}
              subagentMessages={subagentMessages}
              expanded={expandedToolCalls.has(toolCall.toolUseId)}
              onToggle={() => onToggleToolCall(toolCall.toolUseId || '')}
            />
          )
        }

        return (
          <ToolCallComponent
            key={toolCall.toolUseId}
            taskId={taskId}
            parentMessageUuid={parentMessageUuid}
            content={toolCall}
            toolResult={toolCall.toolUseId ? toolResultMap.get(toolCall.toolUseId) : undefined}
            expanded={expandedToolCalls.has(toolCall.toolUseId || '')}
            onToggle={() => onToggleToolCall(toolCall.toolUseId || '')}
            hasPendingUserInput={hasPendingUserInput}
          />
        )
      })}
    </div>
  )
}

/**
 * Render a collapsed message view (for hidden/transcript-only messages)
 */
function renderCollapsedMessage(
  taskId: string | undefined,
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
  channel: UiMessage['channel'] | null | undefined,
  subagentMessages?: UiMessage[],
  parentMessageUuid?: string,
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
      {channel && <ChannelChip channel={channel} />}
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
              taskId,
              toolCalls,
              toolResultMap,
              expandedToolCalls,
              onToggleToolCall,
              hasPendingUserInput,
              subagentMessages,
              parentMessageUuid,
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function CodexErrorCard({ text }: { text: string }) {
  return (
    <div className="border border-red-500/40 rounded-lg bg-red-500/5 p-3 flex items-start gap-2">
      <AlertTriangle className="h-4 w-4 flex-shrink-0 text-red-500 mt-0.5" />
      <div className="text-sm text-red-600 dark:text-red-400 whitespace-pre-wrap break-words">
        {text}
      </div>
    </div>
  )
}

function CodexReasoningCard({ text }: { text: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="border border-dashed border-muted-foreground/30 rounded-lg overflow-hidden bg-muted/10">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full flex items-center gap-2 p-2 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
      >
        {open ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0" />
        )}
        <Brain className="h-3 w-3 flex-shrink-0" />
        <span className="font-medium">Reasoning</span>
        {!open && (
          <span className="truncate text-muted-foreground/70">
            {text.slice(0, 80)}
            {text.length > 80 ? '...' : ''}
          </span>
        )}
      </button>
      {open && (
        <div className="p-3 border-t border-dashed border-muted-foreground/20 text-sm">
          <Response>{text}</Response>
        </div>
      )}
    </div>
  )
}

export function AssistantMessageComponent({
  taskId,
  message,
  expandedToolCalls,
  onToggleToolCall,
}: AssistantMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = (message.messageType as string) === 'transcript_only'
  const isSummary = (message.messageType as string) === 'compact_summary'
  const isCodexReasoning = message.sdkMetadata?.codex_reasoning === true
  const isCodexError = message.sdkMetadata?.codex_error === true

  // Build a lookup map from toolUseId to toolResult
  const toolResultMap = buildToolResultMap(message)

  // Content is always UiContentBlock[]
  const { textContent, toolCalls } = parseMessageContent(message.content)

  // Codex reasoning messages are rendered as a collapsible card instead of
  // inline assistant text.
  if (isCodexReasoning && textContent && toolCalls.length === 0) {
    return <CodexReasoningCard text={textContent} />
  }

  // Non-fatal codex error items get a distinct red card so the user notices
  // them; the turn itself is still running or completed normally.
  if (isCodexError && textContent && toolCalls.length === 0) {
    return <CodexErrorCard text={textContent} />
  }

  // For hidden messages, show collapsed view by default
  if (isHidden) {
    const previewText =
      textContent || (toolCalls.length > 0 ? `${toolCalls.length} tool calls` : '')

    return renderCollapsedMessage(
      taskId,
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
      message.channel,
      message.subagentMessages,
      message.uuid,
    )
  }

  // Normal message display
  return (
    <div className="space-y-2 w-full max-w-full">
      {message.channel && <ChannelChip channel={message.channel} />}

      {/* Text content */}
      {textContent && (
        <div className="text-sm w-full max-w-full">
          <Response>{textContent}</Response>
          {taskId && (
            <AssistantMessageFeedbackIndicator taskId={taskId} messageUuid={message.uuid} />
          )}
        </div>
      )}

      {/* Tool calls */}
      {renderToolCalls(
        taskId,
        toolCalls,
        toolResultMap,
        expandedToolCalls,
        onToggleToolCall,
        message.hasPendingUserInput,
        message.subagentMessages,
        message.uuid,
      )}
    </div>
  )
}

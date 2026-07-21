import type { UiMessage } from '@moatless/api-client'
import { MessageSquare } from 'lucide-react'
import { useState } from 'react'
import { useTaskFeedbackIndex } from '@/features/feedback/hooks/use-task-feedback'
import { Response } from '@/lib/components/ai-elements/response'
import { CodexErrorCard, CodexReasoningCard } from './codex-cards.tsx'
import { renderCollapsedMessage } from './collapsed-message.tsx'
import { ExternalSourceChip } from './external-source-chip.tsx'
import { buildToolResultMap, parseMessageContent } from './message-content.ts'
import { renderToolCalls } from './tool-calls/tool-call-list.tsx'

interface AssistantMessageProps {
  taskId?: string | undefined
  message: UiMessage
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
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

export function AssistantMessageComponent({
  taskId,
  message,
  expandedToolCalls,
  onToggleToolCall,
}: AssistantMessageProps) {
  const [isExpanded, setIsExpanded] = useState(false)
  const isHidden = (message.messageType as string) === 'transcript_only'
  const isSummary = (message.messageType as string) === 'compact_summary'
  const isCodexReasoning = message.sdkMetadata?.['codex_reasoning'] === true
  const isCodexError = message.sdkMetadata?.['codex_error'] === true

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
      message.externalSource,
      message.uuid,
    )
  }

  // Normal message display
  return (
    <div className="space-y-2 w-full max-w-full">
      {message.externalSource && <ExternalSourceChip source={message.externalSource} />}

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
        message.uuid,
      )}
    </div>
  )
}

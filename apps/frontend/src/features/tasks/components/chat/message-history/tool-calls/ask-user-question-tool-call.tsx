import type { UiToolResult } from '@moatless/api-client'
import { CheckCircle, ChevronDown, ChevronRight, HelpCircle } from 'lucide-react'
import type { Question } from '@/lib/types/user-input'
import { cn } from '@/lib/utils'
import type { ToolCallProps } from './tool-call-props.ts'

/**
 * Parse answers from the tool result - try JSON first, fallback to the raw
 * text content.
 */
function parseAnswers(toolResult: UiToolResult | undefined): {
  answers?: Record<string, string>
  answerText?: string
} {
  if (!toolResult?.content) {
    return {}
  }
  try {
    const result = JSON.parse(toolResult.content)
    if (result.answers && typeof result.answers === 'object') {
      return { answers: result.answers }
    }
    return {}
  } catch {
    // Not JSON - use raw text as fallback
    return { answerText: toolResult.content }
  }
}

/**
 * Completed-question summary for the AskUserQuestion tool. Pending questions
 * (no result yet) are rendered in the input area instead, so this returns
 * `null` for them.
 */
export function AskUserQuestionToolCall({
  content,
  toolResult,
  expanded,
  onToggle,
  hasPendingUserInput,
}: ToolCallProps) {
  const toolInput = content.toolInput as { questions: Question[] }
  const questions = toolInput.questions

  // If pending (will be rendered in input area), skip rendering here
  if (hasPendingUserInput && !toolResult) {
    return null
  }

  const { answers, answerText } = parseAnswers(toolResult)
  const hasResponse = !!answers || !!answerText
  const isExpandable = hasResponse

  // Show as completed question summary
  const questionCount = questions.length
  const firstQuestion = questions[0]?.question || ''
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
                ? questions.map((q) => (
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

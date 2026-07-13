import type { UiMessage } from '@moatless/api-client'
import { useMemo } from 'react'
import type { Question } from '@/lib/types/user-input'

export type PendingUserInput =
  | { type: 'AskUserQuestion'; questions: Question[]; toolUseId: string }
  | { type: 'ExitPlanMode'; plan: string | undefined; toolUseId: string }

/**
 * Detect pending user input from the last message: the last unresolved
 * AskUserQuestion / ExitPlanMode tool call on a trailing assistant message
 * that is flagged `hasPendingUserInput`.
 */
export function usePendingUserInput(messages: UiMessage[] | undefined): PendingUserInput | null {
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex nested logic to extract pending tool info
  return useMemo(() => {
    if (!messages || messages.length === 0) {
      return null
    }

    const lastMessage = messages.at(-1)
    if (!lastMessage || lastMessage.role !== 'assistant') {
      return null
    }

    if (!lastMessage.hasPendingUserInput) {
      return null
    }

    // Find the pending tool call (last tool_use without a result in toolResults)
    const toolResultIds = new Set(
      (lastMessage.toolResults ?? []).map((tr) => tr.toolUseId).filter(Boolean),
    )

    for (let i = lastMessage.content.length - 1; i >= 0; i--) {
      const content = lastMessage.content[i]
      if (!content) {
        continue
      }
      if (
        content.type === 'tool_use' &&
        content.toolUseId &&
        !toolResultIds.has(content.toolUseId)
      ) {
        if (content.toolName === 'AskUserQuestion' && content.toolInput) {
          const toolInput = content.toolInput as { questions?: Question[] }
          if (toolInput.questions && Array.isArray(toolInput.questions)) {
            return {
              type: 'AskUserQuestion' as const,
              questions: toolInput.questions,
              toolUseId: content.toolUseId,
            }
          }
        }
        if (content.toolName === 'ExitPlanMode' && content.toolInput) {
          const toolInput = content.toolInput as { plan?: string }
          return {
            type: 'ExitPlanMode' as const,
            plan: toolInput.plan,
            toolUseId: content.toolUseId,
          }
        }
      }
    }

    return null
  }, [messages])
}

import type { UiMessage } from '@moatless/api-client'
import type { Question } from '@/lib/types/user-input'

export type PendingUserInput =
  | { type: 'AskUserQuestion'; questions: Question[]; toolUseId: string }
  | { type: 'ExitPlanMode'; plan: string | undefined; toolUseId: string }

function pendingInputFromContent(
  content: UiMessage['content'][number],
  toolResultIds: Set<string>,
): PendingUserInput | null {
  if (
    content.type !== 'tool_use' ||
    !content.toolUseId ||
    toolResultIds.has(content.toolUseId) ||
    !content.toolInput
  ) {
    return null
  }

  if (content.toolName === 'AskUserQuestion') {
    const toolInput = content.toolInput as { questions?: Question[] }
    if (Array.isArray(toolInput.questions)) {
      return {
        type: 'AskUserQuestion',
        questions: toolInput.questions,
        toolUseId: content.toolUseId,
      }
    }
  }

  if (content.toolName === 'ExitPlanMode') {
    const toolInput = content.toolInput as { plan?: string }
    return {
      type: 'ExitPlanMode',
      plan: toolInput.plan,
      toolUseId: content.toolUseId,
    }
  }

  return null
}

/**
 * Detect pending user input from the last message: the last unresolved
 * AskUserQuestion / ExitPlanMode tool call on a trailing assistant message
 * that is flagged `hasPendingUserInput`.
 */
export function usePendingUserInput(messages: UiMessage[] | undefined): PendingUserInput | null {
  if (!messages || messages.length === 0) {
    return null
  }

  const lastMessage = messages.at(-1)
  if (lastMessage?.role !== 'assistant') {
    return null
  }

  if (!lastMessage.hasPendingUserInput) {
    return null
  }

  // Find the pending tool call (last tool_use without a result in toolResults)
  const toolResultIds = new Set(
    (lastMessage.toolResults ?? []).flatMap((tr) => (tr.toolUseId ? [tr.toolUseId] : [])),
  )

  for (let i = lastMessage.content.length - 1; i >= 0; i--) {
    const content = lastMessage.content[i]
    if (!content) {
      continue
    }
    const pendingInput = pendingInputFromContent(content, toolResultIds)
    if (pendingInput) {
      return pendingInput
    }
  }

  return null
}

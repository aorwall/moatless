import type { AssistantMessage, AssistantMessageContent, Message } from '@moatless/types'

/**
 * Groups consecutive assistant messages that contain only tool calls.
 * This helps reduce visual clutter in the message history.
 *
 * @param message
 * @returns Array of messages and grouped message arrays
 */
// Helper: Check if message contains only tool calls
function hasOnlyToolCalls(message: AssistantMessage): boolean {
  return (
    typeof message.content !== 'string' &&
    Array.isArray(message.content) &&
    message.content.length > 0 &&
    message.content.every((c: AssistantMessageContent) => c.type === 'tool_use')
  )
}

// Helper: Check if message should be grouped
function shouldGroupMessage(
  index: number,
  initialMessageCount: number | null,
  totalMessages: number,
): boolean {
  if (initialMessageCount === null) {
    return false
  }

  return index < initialMessageCount || initialMessageCount === totalMessages
}

// Helper: Push accumulated tool group to groups array
// Always push as array to ensure consistent grouped UI even for single tool calls
function pushToolGroup(
  groups: (Message | AssistantMessage[])[],
  currentToolGroup: AssistantMessage[],
): void {
  if (currentToolGroup.length > 0) {
    groups.push(currentToolGroup)
  }
}

// Helper: Process user message
function processUserMessage(
  message: Message,
  groups: (Message | AssistantMessage[])[],
  currentToolGroup: AssistantMessage[],
): AssistantMessage[] {
  pushToolGroup(groups, currentToolGroup)
  groups.push(message)
  return []
}

// Helper: Process assistant message
function processAssistantMessage(
  message: Message,
  index: number,
  initialMessageCount: number | null,
  totalMessages: number,
  groups: (Message | AssistantMessage[])[],
  currentToolGroup: AssistantMessage[],
): AssistantMessage[] {
  const assistantMsg = message as AssistantMessage
  const onlyToolCalls = hasOnlyToolCalls(assistantMsg)
  const shouldGroup = shouldGroupMessage(index, initialMessageCount, totalMessages)

  if (onlyToolCalls && shouldGroup) {
    return [...currentToolGroup, assistantMsg]
  }

  // This message has text content or shouldn't be grouped
  pushToolGroup(groups, currentToolGroup)
  groups.push(assistantMsg)
  return []
}

export function groupMessages(
  messages: Message[],
  initialMessageCount: number | null,
): (Message | AssistantMessage[])[] {
  if (!messages || messages.length === 0) {
    return []
  }

  const groups: (Message | AssistantMessage[])[] = []
  let currentToolGroup: AssistantMessage[] = []

  for (const [index, message] of messages.entries()) {
    if (message.role === 'user') {
      currentToolGroup = processUserMessage(message, groups, currentToolGroup)
    } else {
      currentToolGroup = processAssistantMessage(
        message,
        index,
        initialMessageCount,
        messages.length,
        groups,
        currentToolGroup,
      )
    }
  }

  // Push any remaining tool group
  pushToolGroup(groups, currentToolGroup)

  return groups
}

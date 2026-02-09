import type { UiContentBlock, UiMessage } from '@moatless/api-client'

/**
 * Groups consecutive assistant messages that contain only tool calls.
 * This helps reduce visual clutter in the message history.
 *
 * @param message
 * @returns Array of messages and grouped message arrays
 */
// Helper: Check if message contains only tool calls
function hasOnlyToolCalls(message: UiMessage): boolean {
  return (
    Array.isArray(message.content) &&
    message.content.length > 0 &&
    message.content.every((c: UiContentBlock) => c.type === 'tool_use') &&
    // Don't group messages containing ExitPlanMode - they should display prominently
    !message.content.some(
      (c: UiContentBlock) => c.type === 'tool_use' && c.toolName === 'ExitPlanMode',
    )
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
function pushToolGroup(groups: (UiMessage | UiMessage[])[], currentToolGroup: UiMessage[]): void {
  if (currentToolGroup.length > 0) {
    groups.push(currentToolGroup)
  }
}

// Helper: Process user message
function processUserMessage(
  message: UiMessage,
  groups: (UiMessage | UiMessage[])[],
  currentToolGroup: UiMessage[],
): UiMessage[] {
  pushToolGroup(groups, currentToolGroup)
  groups.push(message)
  return []
}

// Helper: Process assistant message
function processAssistantMessage(
  message: UiMessage,
  index: number,
  initialMessageCount: number | null,
  totalMessages: number,
  groups: (UiMessage | UiMessage[])[],
  currentToolGroup: UiMessage[],
): UiMessage[] {
  const onlyToolCalls = hasOnlyToolCalls(message)
  const shouldGroup = shouldGroupMessage(index, initialMessageCount, totalMessages)

  if (onlyToolCalls && shouldGroup) {
    return [...currentToolGroup, message]
  }

  // This message has text content or shouldn't be grouped
  pushToolGroup(groups, currentToolGroup)
  groups.push(message)
  return []
}

export function groupMessages(
  messages: UiMessage[],
  initialMessageCount: number | null,
): (UiMessage | UiMessage[])[] {
  if (!messages || messages.length === 0) {
    return []
  }

  const groups: (UiMessage | UiMessage[])[] = []
  let currentToolGroup: UiMessage[] = []

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

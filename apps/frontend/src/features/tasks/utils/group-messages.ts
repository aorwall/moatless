import type { AssistantMessage, AssistantMessageContent, Message } from '@moatless/types'

/**
 * Groups consecutive assistant messages that contain only tool calls.
 * This helps reduce visual clutter in the message history.
 *
 * @param messages - Array of messages to group
 * @param initialMessageCount - Number of messages that were present on initial load (for grouping logic)
 * @returns Array of messages and grouped message arrays
 */
export function groupMessages(
  messages: Message[],
  initialMessageCount: number | null,
): (Message | AssistantMessage[])[] {
  if (!messages || messages.length === 0) {
    return []
  }

  const groups: (Message | AssistantMessage[])[] = []
  let currentToolGroup: AssistantMessage[] = []
  const shouldGroupMessages = initialMessageCount !== null

  messages.forEach((message, index) => {
    if (message.role === 'user') {
      // If we have accumulated tool messages, push them as a group (only if more than 1)
      if (currentToolGroup.length > 1) {
        groups.push(currentToolGroup)
      } else if (currentToolGroup.length === 1) {
        groups.push(currentToolGroup[0])
      }
      currentToolGroup = []
      groups.push(message)
    } else {
      const assistantMsg = message as AssistantMessage
      // Check if this message contains only tool calls (no text content)
      const hasOnlyToolCalls =
        typeof assistantMsg.content !== 'string' &&
        Array.isArray(assistantMsg.content) &&
        assistantMsg.content.length > 0 &&
        assistantMsg.content.every((c: AssistantMessageContent) => c.type === 'tool_use')

      // Only group messages that were present on mount OR new messages if view is open
      const shouldGroup =
        shouldGroupMessages &&
        (index < (initialMessageCount || 0) || initialMessageCount === messages.length)

      if (hasOnlyToolCalls && shouldGroup) {
        currentToolGroup.push(assistantMsg)
      } else {
        // This message has text content or shouldn't be grouped
        // Push any accumulated tool group first (only if more than 1 message)
        if (currentToolGroup.length > 1) {
          groups.push(currentToolGroup)
        } else if (currentToolGroup.length === 1) {
          // Single tool call message - don't group
          groups.push(currentToolGroup[0])
        }
        currentToolGroup = []
        groups.push(assistantMsg)
      }
    }
  })

  // Push any remaining tool group (only if more than 1 message)
  if (currentToolGroup.length > 1) {
    groups.push(currentToolGroup)
  } else if (currentToolGroup.length === 1) {
    // Single tool call message - don't group
    groups.push(currentToolGroup[0])
  }

  return groups
}

import type { UiContentBlock, UiMessage } from '@moatless/api-client'

/**
 * Extended UiMessage type that includes subagent messages.
 */
export interface UiMessageWithSubagents extends UiMessage {
  subagentMessages?: UiMessage[]
}

/**
 * Extracts all Agent tool call IDs from messages.
 */
function collectAgentToolIds(messages: UiMessage[]): Set<string> {
  const agentToolIds = new Set<string>()
  for (const msg of messages) {
    for (const block of msg.content) {
      if (block.type === 'tool_use' && block.toolName === 'Agent' && block.toolUseId) {
        agentToolIds.add(block.toolUseId)
      }
    }
  }
  return agentToolIds
}

/**
 * Partitions messages into main messages and a map of subagent messages.
 */
function partitionMessages(
  messages: UiMessage[],
  agentToolIds: Set<string>,
): {
  mainMessages: UiMessageWithSubagents[]
  subagentMap: Map<string, UiMessage[]>
} {
  const subagentMap = new Map<string, UiMessage[]>()
  const mainMessages: UiMessageWithSubagents[] = []

  for (const msg of messages) {
    if (msg.parentUuid && agentToolIds.has(msg.parentUuid)) {
      const list = subagentMap.get(msg.parentUuid) ?? []
      list.push(msg)
      subagentMap.set(msg.parentUuid, list)
    } else {
      mainMessages.push({ ...msg })
    }
  }

  return { mainMessages, subagentMap }
}

/**
 * Attaches subagent messages to their parent Agent tool calls.
 */
function attachSubagentMessages(
  mainMessages: UiMessageWithSubagents[],
  subagentMap: Map<string, UiMessage[]>,
): void {
  for (const msg of mainMessages) {
    for (const block of msg.content) {
      if (block.type === 'tool_use' && block.toolName === 'Agent' && block.toolUseId) {
        const subagentMsgs = subagentMap.get(block.toolUseId)
        if (subagentMsgs && subagentMsgs.length > 0) {
          msg.subagentMessages = subagentMsgs
        }
      }
    }
  }
}

/**
 * Groups subagent messages under their parent Agent tool calls.
 * Returns messages with subagent tool calls attached to the parent message.
 *
 * @param messages - Array of UI messages
 * @returns Array of messages with subagent messages grouped under parent Agent tool calls
 */
export function groupSubagentMessages(messages: UiMessage[]): UiMessageWithSubagents[] {
  if (!messages || messages.length === 0) {
    return []
  }

  const agentToolIds = collectAgentToolIds(messages)

  if (agentToolIds.size === 0) {
    return messages as UiMessageWithSubagents[]
  }

  const { mainMessages, subagentMap } = partitionMessages(messages, agentToolIds)
  attachSubagentMessages(mainMessages, subagentMap)

  return mainMessages
}

/**
 * Groups consecutive assistant messages that contain only tool calls.
 * This helps reduce visual clutter in the message history.
 *
 * @param message
 * @returns Array of messages and grouped message arrays
 */
// Helper: Check if message contains only tool calls
function hasOnlyToolCalls(message: UiMessageWithSubagents): boolean {
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
function pushToolGroup(
  groups: (UiMessageWithSubagents | UiMessageWithSubagents[])[],
  currentToolGroup: UiMessageWithSubagents[],
): void {
  if (currentToolGroup.length > 0) {
    groups.push(currentToolGroup)
  }
}

// Helper: Process user message
function processUserMessage(
  message: UiMessageWithSubagents,
  groups: (UiMessageWithSubagents | UiMessageWithSubagents[])[],
  currentToolGroup: UiMessageWithSubagents[],
): UiMessageWithSubagents[] {
  pushToolGroup(groups, currentToolGroup)
  groups.push(message)
  return []
}

// Helper: Process assistant message
function processAssistantMessage(
  message: UiMessageWithSubagents,
  index: number,
  initialMessageCount: number | null,
  totalMessages: number,
  groups: (UiMessageWithSubagents | UiMessageWithSubagents[])[],
  currentToolGroup: UiMessageWithSubagents[],
): UiMessageWithSubagents[] {
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
  messages: UiMessageWithSubagents[],
  initialMessageCount: number | null,
): (UiMessageWithSubagents | UiMessageWithSubagents[])[] {
  if (!messages || messages.length === 0) {
    return []
  }

  const groups: (UiMessageWithSubagents | UiMessageWithSubagents[])[] = []
  let currentToolGroup: UiMessageWithSubagents[] = []

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

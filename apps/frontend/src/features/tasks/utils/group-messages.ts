import type { UiMessage } from '@moatless/api-client'

/**
 * Extended UiMessage type that includes subagent messages.
 *
 * Keyed by the spawning Agent tool call: one assistant message can launch
 * several subagents in parallel, and each tool call renders its own transcript.
 */
export interface UiMessageWithSubagents extends UiMessage {
  subagentsByToolUseId?: Record<string, UiMessage[]>
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
          msg.subagentsByToolUseId = {
            ...msg.subagentsByToolUseId,
            [block.toolUseId]: subagentMsgs,
          }
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

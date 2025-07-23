import type {
  AgentEvent,
  ClaudeCodeSDKMessage,
  Event,
  SDKAssistantMessage,
  SDKUserMessage,
} from '@moatless/types'
import { isToolResultContent, isToolUseContent } from '@moatless/types'

export interface MergedEvent extends Event {
  toolResult?: Event
}

function isAgentMessageEvent(event: Event): event is AgentEvent {
  return event.type === 'agent' && event.subType === 'message'
}

function hasAssistantMessage(
  payload: ClaudeCodeSDKMessage,
): payload is { message: SDKAssistantMessage } & ClaudeCodeSDKMessage {
  return 'message' in payload && payload.message?.role === 'assistant'
}

function hasUserMessage(
  payload: ClaudeCodeSDKMessage,
): payload is { message: SDKUserMessage } & ClaudeCodeSDKMessage {
  return 'message' in payload && payload.message?.role === 'user'
}

export function mergeToolEvents(events: Event[]): (Event | MergedEvent)[] {
  // Early return for empty arrays
  if (events.length === 0) return []

  const toolResultsMap = new Map<string, Event>()
  const processedEventIds = new Set<string>()

  // First pass: collect all tool results by tool_use_id
  for (const event of events) {
    if (isAgentMessageEvent(event)) {
      // Check if payload structure is as expected
      if (!event.payload || typeof event.payload !== 'object') {
        console.warn(
          `[mergeToolEvents] MessageEvent ${event.eventId} has invalid payload structure`,
          event,
        )
        continue
      }

      if (hasUserMessage(event.payload as any)) {
        const { message } = event.payload

        if (message.content && Array.isArray(message.content)) {
          for (const content of message.content) {
            if (isToolResultContent(content)) {
              toolResultsMap.set(content.tool_use_id, event)
            }
          }
        }
      }
    }
  }

  // Second pass: merge tool calls with their results
  const mergedEvents: (Event | MergedEvent)[] = []
  const unmatchedToolCalls: string[] = []

  for (const event of events) {
    // Skip if already processed as a tool result
    if (processedEventIds.has(event.eventId)) {
      continue
    }

    if (isAgentMessageEvent(event)) {
      // Check if payload structure is as expected
      if (!event.payload || typeof event.payload !== 'object') {
        console.warn(
          `[mergeToolEvents] MessageEvent ${event.eventId} has invalid payload structure`,
          event,
        )
        mergedEvents.push(event)
        continue
      }

      if (hasAssistantMessage(event.payload as any)) {
        const { message } = event.payload
        let hasToolUse = false
        let processedAsToolCall = false

        if (message.content && Array.isArray(message.content)) {
          // Check if this message contains any tool_use content
          hasToolUse = message.content.some(isToolUseContent)

          if (hasToolUse) {
            // Process each tool_use content
            for (const content of message.content) {
              if (isToolUseContent(content)) {
                const toolUseId = content.id
                const toolResult = toolResultsMap.get(toolUseId)

                if (toolResult) {
                  // Mark the tool result as processed
                  processedEventIds.add(toolResult.eventId)

                  // Create merged event with unique ID
                  const mergedEvent: MergedEvent = {
                    ...event,
                    // Create unique eventId by combining original ID with tool use ID
                    eventId: `${event.eventId}-${toolUseId}`,
                    toolResult,
                  }
                  mergedEvents.push(mergedEvent)
                  processedAsToolCall = true
                } else {
                  // Track unmatched tool calls
                  unmatchedToolCalls.push(toolUseId)
                }
              }
            }

            // If we didn't process it as a merged event, add as-is
            if (!processedAsToolCall) {
              mergedEvents.push(event)
            }
          } else {
            // No tool_use content, add as-is
            mergedEvents.push(event)
          }
        } else {
          // No content array, add as-is
          mergedEvents.push(event)
        }
      } else if (!hasUserMessage(event.payload as any)) {
        // Message event but neither assistant nor user type

        mergedEvents.push(event)
      } else {
        // User message that wasn't a tool result (already processed above)
        mergedEvents.push(event)
      }
    } else {
      // Not a message event, add as-is
      mergedEvents.push(event)
    }
  }

  // Log summary of unmatched tool calls if any
  if (unmatchedToolCalls.length > 0) {
    console.warn(
      `[mergeToolEvents] Found ${unmatchedToolCalls.length} tool calls without matching results:`,
      unmatchedToolCalls,
    )
  }

  // Log orphaned tool results
  const orphanedResults = Array.from(toolResultsMap.entries()).filter(
    ([_, event]) => !processedEventIds.has(event.eventId),
  )
  if (orphanedResults.length > 0) {
    console.warn(
      `[mergeToolEvents] Found ${orphanedResults.length} orphaned tool results:`,
      orphanedResults.map(([toolUseId]) => toolUseId),
    )
  }

  return mergedEvents
}

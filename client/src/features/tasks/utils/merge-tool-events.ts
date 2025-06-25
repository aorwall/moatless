import type {
  ClaudeCodeSDKMessage,
  SDKAssistantMessage,
  SDKUserMessage,
} from '@shared/types/claude'
import { isToolResultContent, isToolUseContent } from '@shared/types/claude-content'
import type { Event, MessageEvent } from '@shared/types/event'

export interface MergedEvent extends Event {
  toolResult?: Event
}

function isMessageEvent(event: Event): event is MessageEvent {
  return event.type === 'message'
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
    if (isMessageEvent(event)) {
      // Check if payload structure is as expected
      if (!event.payload || typeof event.payload !== 'object') {
        console.warn(
          `[mergeToolEvents] MessageEvent ${event.eventId} has invalid payload structure`,
          event,
        )
        continue
      }

      if (hasUserMessage(event.payload)) {
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

    if (isMessageEvent(event)) {
      // Check if payload structure is as expected
      if (!event.payload || typeof event.payload !== 'object') {
        console.warn(
          `[mergeToolEvents] MessageEvent ${event.eventId} has invalid payload structure`,
          event,
        )
        mergedEvents.push(event)
        continue
      }

      if (hasAssistantMessage(event.payload)) {
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

                  // Create merged event
                  const mergedEvent: MergedEvent = {
                    ...event,
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
      } else if (!hasUserMessage(event.payload)) {
        // Message event but neither assistant nor user type
        console.warn(
          `[mergeToolEvents] MessageEvent ${event.eventId} has unexpected message type`,
          event.payload,
        )
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

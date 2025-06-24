import type { Event } from '@shared/types/event'

export interface MergedEvent extends Event {
  toolResult?: Event
}

export function mergeToolEvents(events: Event[]): (Event | MergedEvent)[] {
  // Early return for empty arrays
  if (events.length === 0) return []

  const toolResultsMap = new Map<string, Event>()
  const processedEventIds = new Set<string>()

  // First pass: collect all tool results by tool_use_id
  for (const event of events) {
    if (event.type === 'message' && !event.subType) {
      const payload = event.payload as Record<string, unknown>
      const message = payload.message as Record<string, unknown> | null

      if (message?.content && Array.isArray(message.content)) {
        for (const content of message.content) {
          if (
            content &&
            typeof content === 'object' &&
            'type' in content &&
            content.type === 'tool_result' &&
            'tool_use_id' in content &&
            typeof content.tool_use_id === 'string'
          ) {
            toolResultsMap.set(content.tool_use_id, event)
          }
        }
      }
    }
  }

  // Second pass: merge tool calls with their results
  const mergedEvents: (Event | MergedEvent)[] = []

  for (const event of events) {
    // Skip if already processed as a tool result
    if (processedEventIds.has(event.eventId)) {
      continue
    }

    if (event.type === 'message' && event.subType === 'tool_call') {
      const payload = event.payload as Record<string, unknown>
      const message = payload.message as Record<string, unknown> | null

      if (message?.content && Array.isArray(message.content)) {
        for (const content of message.content) {
          if (
            content &&
            typeof content === 'object' &&
            'type' in content &&
            content.type === 'tool_use' &&
            'id' in content &&
            typeof content.id === 'string'
          ) {
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
            } else {
              // No matching result found, add as-is
              mergedEvents.push(event)
            }
          } else {
            // Not a tool use, add as-is
            mergedEvents.push(event)
          }
        }
      } else {
        // No content array, add as-is
        mergedEvents.push(event)
      }
    } else {
      // Not a tool call, add as-is (unless it's already processed as a result)
      mergedEvents.push(event)
    }
  }

  return mergedEvents
}

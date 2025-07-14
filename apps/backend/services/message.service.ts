import type {
  AssistantMessage,
  AssistantMessageContent,
  Event,
  Message,
  MessageEvent,
  ToolResult,
  UserMessage,
} from '@moatless/types'
import {
  type ClaudeCodeSDKMessage,
  isToolResultContent,
  isToolUseContent,
  type SDKAssistantMessage,
  type SDKUserMessage,
} from '@moatless/types'

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

export class MessageService {
  /**
   * Transform events into a list of messages with merged tool calls and results
   */
  transformEventsToMessages(events: Event[]): Message[] {
    if (events.length === 0) return []

    const toolResultsMap = new Map<string, MessageEvent>()
    const processedEventIds = new Set<string>()
    const messages: Message[] = []

    // First pass: collect all tool results by tool_use_id
    for (const event of events) {
      if (isMessageEvent(event)) {
        if (!event.payload || typeof event.payload !== 'object') {
          console.warn(
            `[transformEventsToMessages] MessageEvent ${event.eventId} has invalid payload structure`,
            event,
          )
          continue
        }

        if (hasUserMessage(event.payload)) {
          const sdkMessage = event.payload as SDKUserMessage
          const apiMessage = sdkMessage.message

          if (apiMessage.content && Array.isArray(apiMessage.content)) {
            for (const content of apiMessage.content) {
              if (isToolResultContent(content)) {
                toolResultsMap.set(content.tool_use_id, event)
              }
            }
          }
        }
      }
    }

    // Second pass: transform events into messages
    for (const event of events) {
      // Skip if already processed as a tool result
      if (processedEventIds.has(event.eventId)) {
        continue
      }

      if (isMessageEvent(event)) {
        if (!event.payload || typeof event.payload !== 'object') {
          console.warn(
            `[transformEventsToMessages] MessageEvent ${event.eventId} has invalid payload structure`,
            event,
          )
          continue
        }

        const message = this.transformMessageEvent(event, toolResultsMap, processedEventIds)
        if (message) {
          messages.push(message)
        }
      }
    }

    return messages
  }

  private transformMessageEvent(
    event: MessageEvent,
    toolResultsMap: Map<string, MessageEvent>,
    processedEventIds: Set<string>,
  ): Message | null {
    if (hasAssistantMessage(event.payload)) {
      return this.transformAssistantMessage(event, toolResultsMap, processedEventIds)
    } else if (hasUserMessage(event.payload)) {
      // Only include user messages that are not tool results
      const sdkMessage = event.payload as SDKUserMessage
      const apiMessage = sdkMessage.message
      if (apiMessage.content && Array.isArray(apiMessage.content)) {
        const hasToolResult = apiMessage.content.some(isToolResultContent)
        if (hasToolResult) {
          // Skip tool result messages as they're merged with tool calls
          return null
        }
      }
      return this.transformUserMessage(event)
    }

    return null
  }

  private transformAssistantMessage(
    event: MessageEvent,
    toolResultsMap: Map<string, MessageEvent>,
    processedEventIds: Set<string>,
  ): AssistantMessage {
    const sdkMessage = event.payload as SDKAssistantMessage
    const apiMessage = sdkMessage.message
    const content: AssistantMessageContent[] = []

    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      for (const messageContent of apiMessage.content) {
        if (messageContent.type === 'text') {
          content.push({
            type: 'text',
            text: messageContent.text,
          })
        } else if (isToolUseContent(messageContent)) {
          const toolUseId = messageContent.id
          const toolResult = toolResultsMap.get(toolUseId)

          let toolResultContent: ToolResult | undefined
          if (toolResult) {
            // Mark the tool result as processed
            processedEventIds.add(toolResult.eventId)

            // Extract tool result content
            const toolResultSDK = toolResult.payload as SDKUserMessage
            const toolResultAPI = toolResultSDK.message
            if (toolResultAPI.content && Array.isArray(toolResultAPI.content)) {
              const resultContent = toolResultAPI.content.find(isToolResultContent)
              if (resultContent) {
                toolResultContent = {
                  content: Array.isArray(resultContent.content)
                    ? resultContent.content
                        .map((c: any) => (c.type === 'text' ? c.text : JSON.stringify(c)))
                        .join('\n')
                    : typeof resultContent.content === 'string'
                      ? resultContent.content
                      : JSON.stringify(resultContent.content),
                  is_error: resultContent.is_error,
                  eventId: toolResult.eventId,
                }
              }
            }
          }

          content.push({
            type: 'tool_use',
            tool_name: messageContent.name,
            tool_input: messageContent.input,
            tool_use_id: toolUseId,
            tool_result: toolResultContent,
          })
        }
      }
    }

    return {
      id: event.eventId,
      role: 'assistant',
      content:
        content.length === 1 && content[0].type === 'text' && content[0].text
          ? content[0].text
          : content,
      timestamp: event.timestamp || new Date(),
      eventId: event.eventId,
    }
  }

  private transformUserMessage(event: MessageEvent): UserMessage {
    const sdkMessage = event.payload as SDKUserMessage
    const apiMessage = sdkMessage.message

    // Extract text content from user message
    let textContent = ''
    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      textContent = apiMessage.content
        .filter((c: any) => c.type === 'text')
        .map((c: any) => c.text)
        .join('\n')
    }

    return {
      id: event.eventId,
      role: 'user',
      content: textContent,
      timestamp: event.timestamp || new Date(),
      eventId: event.eventId,
    }
  }
}

export function createMessageService(): MessageService {
  return new MessageService()
}

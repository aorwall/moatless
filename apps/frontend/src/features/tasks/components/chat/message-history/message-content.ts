import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'

/**
 * Build a lookup map from toolUseId to toolResult for efficient retrieval
 */
export function buildToolResultMap(message: UiMessage): Map<string, UiToolResult> {
  const toolResultMap = new Map<string, UiToolResult>()
  if (message.toolResults) {
    for (const tr of message.toolResults) {
      if (tr.toolUseId) {
        toolResultMap.set(tr.toolUseId, tr)
      }
    }
  }
  return toolResultMap
}

/**
 * Separate content array into text content and tool calls
 */
export function parseMessageContent(contentArray: UiContentBlock[]): {
  textContent: string
  toolCalls: UiContentBlock[]
} {
  const textParts: string[] = []
  const toolCalls: UiContentBlock[] = []

  for (const content of contentArray) {
    if (content.type === 'text') {
      textParts.push(content.text ?? '')
    } else if (content.type === 'tool_use') {
      toolCalls.push(content)
    }
  }

  return { textContent: textParts.join('\n'), toolCalls }
}

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
  const textContent = contentArray
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')
  const toolCalls = contentArray.filter((c) => c.type === 'tool_use')
  return { textContent, toolCalls }
}

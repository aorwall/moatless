import type { UiMessage } from '@moatless/api-client'

export const MAX_PREVIEW_LENGTH = 140

/** Collapse whitespace and clamp text to a single sidebar-friendly line. */
export function formatPreview(text: string): string {
  const collapsed = text.replace(/\s+/g, ' ').trim()
  return collapsed.length > MAX_PREVIEW_LENGTH
    ? `${collapsed.slice(0, MAX_PREVIEW_LENGTH)}…`
    : collapsed
}

/** A message reduced to what the sidebar needs: a text preview and its time. */
export interface MessagePreview {
  text: string
  at: string
}

/**
 * Reduce a page of UI messages (oldest-first) to a preview of the most recent
 * one worth showing: the newest message with prose, or failing that the newest
 * tool call surfaced as `Using <tool>…`. Returns `null` when nothing qualifies.
 */
export function previewFromUiMessages(messages: UiMessage[]): MessagePreview | null {
  // Prose is preferred over tool calls, so scan newest-first for the last
  // message with text and only keep the newest tool call as a fallback for
  // tasks whose recent messages are all tool activity.
  let toolFallback: MessagePreview | null = null

  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]
    if (!message) {
      continue
    }
    const texts: string[] = []
    let toolName: string | null = null

    for (const block of message.content) {
      if (block.type === 'text' && block.text?.trim()) {
        texts.push(block.text)
      } else if (block.toolName && !toolName) {
        toolName = block.toolName
      }
    }

    const joined = texts.join(' ').trim()
    if (joined.length > 0) {
      return { text: formatPreview(joined), at: message.timestamp }
    }
    if (toolName && !toolFallback) {
      toolFallback = { text: `Using ${toolName}…`, at: message.timestamp }
    }
  }

  return toolFallback
}

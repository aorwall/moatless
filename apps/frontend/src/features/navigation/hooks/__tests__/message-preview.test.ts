import type { UiMessage } from '@moatless/api-client'
import { describe, expect, it } from 'vitest'
import { formatPreview, previewFromUiMessages } from '../message-preview'

/** Build a minimal UiMessage carrying the given content blocks. */
function uiMessage(content: UiMessage['content'], timestamp = '2026-07-08T00:00:00Z'): UiMessage {
  return {
    content,
    role: 'assistant',
    messageType: 'assistant',
    timestamp,
    uuid: `m-${timestamp}`,
  } as unknown as UiMessage
}

describe('formatPreview', () => {
  it('collapses whitespace', () => {
    expect(formatPreview('a\n\n  b   c')).toBe('a b c')
  })

  it('truncates with an ellipsis', () => {
    const preview = formatPreview('x'.repeat(200))
    expect(preview).toHaveLength(141)
    expect(preview.endsWith('…')).toBe(true)
  })
})

describe('previewFromUiMessages', () => {
  it('returns null for an empty page', () => {
    expect(previewFromUiMessages([])).toBeNull()
  })

  it('uses the newest message with prose', () => {
    const messages = [
      uiMessage([{ type: 'text', text: 'older' }], '2026-07-08T00:00:00Z'),
      uiMessage([{ type: 'text', text: 'newest reply' }], '2026-07-08T00:01:00Z'),
    ]
    expect(previewFromUiMessages(messages)).toEqual({
      text: 'newest reply',
      at: '2026-07-08T00:01:00Z',
    })
  })

  it('skips trailing tool-only rows to find the last prose', () => {
    const messages = [
      uiMessage([{ type: 'text', text: 'the real answer' }], '2026-07-08T00:00:00Z'),
      uiMessage([{ type: 'tool_use', toolName: 'Bash' }], '2026-07-08T00:01:00Z'),
    ]
    expect(previewFromUiMessages(messages)?.text).toBe('the real answer')
  })

  it('falls back to the tool name when no prose exists', () => {
    const messages = [uiMessage([{ type: 'tool_use', toolName: 'Edit' }])]
    expect(previewFromUiMessages(messages)?.text).toBe('Using Edit…')
  })
})

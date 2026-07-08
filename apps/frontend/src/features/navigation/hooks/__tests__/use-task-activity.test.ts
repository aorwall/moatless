import { describe, expect, it } from 'vitest'
import type { Event } from '@/lib/types/events'
import { extractMessagePreview } from '../use-task-activity'

/** Build a minimal agent `message` event carrying the given Claude content. */
function agentMessageEvent(content: unknown): Event {
  return {
    type: 'agent',
    subType: 'message',
    eventId: 'evt-1',
    taskId: 'task-1',
    timestamp: '2026-07-08T00:00:00Z',
    isError: false,
    payload: { type: 'assistant', message: { content } },
  } as unknown as Event
}

describe('extractMessagePreview', () => {
  it('returns null for non agent-message events', () => {
    const event = { type: 'sandbox', subType: 'ready' } as unknown as Event
    expect(extractMessagePreview(event)).toBeNull()
  })

  it('extracts plain string content', () => {
    expect(extractMessagePreview(agentMessageEvent('Hello there'))).toBe('Hello there')
  })

  it('joins text blocks and collapses whitespace', () => {
    const content = [
      { type: 'text', text: 'First line\n\n  second' },
      { type: 'text', text: 'third' },
    ]
    expect(extractMessagePreview(agentMessageEvent(content))).toBe('First line second third')
  })

  it('falls back to the tool name when a message has no prose', () => {
    const content = [{ type: 'tool_use', id: 't1', name: 'Bash', input: {} }]
    expect(extractMessagePreview(agentMessageEvent(content))).toBe('Using Bash…')
  })

  it('prefers text over tool names when both are present', () => {
    const content = [
      { type: 'text', text: 'Running the build' },
      { type: 'tool_use', id: 't1', name: 'Bash', input: {} },
    ]
    expect(extractMessagePreview(agentMessageEvent(content))).toBe('Running the build')
  })

  it('truncates long previews with an ellipsis', () => {
    const long = 'x'.repeat(200)
    const preview = extractMessagePreview(agentMessageEvent(long))
    expect(preview).toHaveLength(141)
    expect(preview?.endsWith('…')).toBe(true)
  })

  it('returns null when there is nothing to show', () => {
    expect(extractMessagePreview(agentMessageEvent([]))).toBeNull()
    expect(extractMessagePreview(agentMessageEvent('   '))).toBeNull()
  })
})

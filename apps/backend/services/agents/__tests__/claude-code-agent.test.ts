import type { SDKMessage } from '@anthropic-ai/claude-code'
import { describe, expect, it, vi } from 'vitest'
import { ClaudeCodeAgent } from '../claude-code-agent.js'

// Mock the @anthropic-ai/claude-code module
vi.mock('@anthropic-ai/claude-code', () => ({
  query: vi.fn().mockImplementation(async function* () {
    // Mock implementation that yields test messages
  }),
}))

describe('ClaudeCodeAgent', () => {
  describe('Tool Call Detection', () => {
    it('should not set subType for regular text messages', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a regular text message
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(async function* () {
        const textMessage: SDKMessage = {
          type: 'assistant',
          message: {
            id: 'msg_123',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'This is a regular text message.',
              },
            ],
            stop_reason: 'end_turn',
            stop_sequence: null,
            usage: {
              input_tokens: 10,
              output_tokens: 20,
            },
          },
          parent_tool_use_id: null,
          session_id: 'test-session',
        }
        yield textMessage
      })

      const events: unknown[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the message event
      const messageEvent = events.find((e: any) => e.type === 'message')
      expect(messageEvent).toBeDefined()
      expect((messageEvent as any).subType).toBeUndefined()
      expect((messageEvent as any).payload.message.content[0].type).toBe('text')
    })
  })
})

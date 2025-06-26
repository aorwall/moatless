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
    it.skip('should set subType to tool_call for assistant messages with tool_use content', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a tool call message
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(async function* () {
        const toolCallMessage: SDKMessage = {
          type: 'assistant',
          message: {
            id: 'msg_01WJ4oGMtXCUrUKfRCcg2z4H',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'tool_use',
                id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                name: 'LS',
                input: {
                  path: '/test/path',
                },
              },
            ],
            stop_reason: null,
            stop_sequence: null,
            usage: {
              input_tokens: 4,
              output_tokens: 6,
            },
          },
          parent_tool_use_id: null,
          session_id: 'test-session',
        }
        yield toolCallMessage
      })

      const events: any[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the message event
      const messageEvent = events.find((e) => e.type === 'message')
      expect(messageEvent).toBeDefined()
      expect(messageEvent.subType).toBe('tool_call')
      expect(messageEvent.payload.message.content[0].type).toBe('tool_use')
    })

    it.skip('should set subType to tool_result for user messages with tool_result content', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a tool result message
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(async function* () {
        const toolResultMessage: SDKMessage = {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                tool_use_id: 'toolu_01WyKnDTfzw1PfJaWyjQ8GUb',
                type: 'tool_result',
                content: 'File listing results here...',
              },
            ],
          },
          parent_tool_use_id: null,
          session_id: 'test-session',
        }
        yield toolResultMessage
      })

      const events: any[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the message event
      const messageEvent = events.find((e) => e.type === 'message')
      expect(messageEvent).toBeDefined()
      expect(messageEvent.subType).toBe('tool_result')
      expect(messageEvent.payload.message.content[0].type).toBe('tool_result')
    })

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

      const events: any[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the message event
      const messageEvent = events.find((e) => e.type === 'message')
      expect(messageEvent).toBeDefined()
      expect(messageEvent.subType).toBeUndefined()
      expect(messageEvent.payload.message.content[0].type).toBe('text')
    })
  })
})

import type { SDKMessage } from '@anthropic-ai/claude-code'
import { describe, expect, it, vi } from 'vitest'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent.js'

// Mock the @anthropic-ai/claude-code module
vi.mock('@anthropic-ai/claude-code', () => ({
  query: vi.fn().mockImplementation(() => {
    const generator = async function* () {
      // Default empty generator - will be overridden in tests
      yield {
        type: 'assistant',
        message: {
          id: 'msg_default',
          type: 'message',
          role: 'assistant',
          model: 'claude-sonnet-4-20250514',
          content: [{ type: 'text', text: 'Default test message' }],
          stop_reason: 'end_turn',
          stop_sequence: null,
        },
        parent_tool_use_id: null,
        session_id: 'default-session',
      }
    }
    const gen = generator()
    return {
      ...gen,
      [Symbol.asyncIterator]: () => gen,
      interrupt: vi.fn(),
    }
  }),
}))

describe('ClaudeCodeAgent', () => {
  describe('Context Tokens Calculation', () => {
    it('should calculate contextTokens from usage data', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a message with usage data
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(() => {
        const generator = async function* () {
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
                  text: 'This is a message with usage data.',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 4,
                output_tokens: 20,
                cache_creation_input_tokens: 99604,
                cache_read_input_tokens: 10161,
              },
            },
            parent_tool_use_id: null,
            session_id: 'test-session',
          }
          yield textMessage
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      const events: unknown[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the agent message event
      const messageEvent = events.find((e: any) => e.type === 'agent' && e.subType === 'message')
      expect(messageEvent).toBeDefined()
      // Verify contextTokens is calculated correctly: 4 + 99604 + 10161 = 109769
      expect((messageEvent as any).contextTokens).toBe(109769)
    })

    it('should not set contextTokens for messages without usage data', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a message without usage data
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(() => {
        const generator = async function* () {
          const textMessage: SDKMessage = {
            type: 'user',
            message: {
              role: 'user',
              content: [
                {
                  type: 'text',
                  text: 'User message without usage data.',
                },
              ],
            },
            parent_tool_use_id: null,
            session_id: 'test-session',
          }
          yield textMessage
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      const events: unknown[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the agent message event
      const messageEvent = events.find((e: any) => e.type === 'agent' && e.subType === 'message')
      expect(messageEvent).toBeDefined()
      // Verify contextTokens is undefined when no usage data
      expect((messageEvent as any).contextTokens).toBeUndefined()
    })
  })

  describe('Tool Call Detection', () => {
    it('should not set subType for regular text messages', async () => {
      const taskId = 'test-task-id'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to yield a regular text message
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(() => {
        const generator = async function* () {
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
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      const events: unknown[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Find the agent message event
      const messageEvent = events.find((e: any) => e.type === 'agent' && e.subType === 'message')
      expect(messageEvent).toBeDefined()
      expect((messageEvent as any).subType).toBe('message')
      expect((messageEvent as any).payload.message.content[0].type).toBe('text')
    })
  })

  describe('Model Parameter Handling', () => {
    it('should use model from runOptions over constructor options', async () => {
      const taskId = 'test-task-id'
      const constructorModel = 'claude-3-opus-20240229'
      const runOptionsModel = 'claude-3-sonnet-20240320'

      // Create agent with model in constructor options
      const agent = new ClaudeCodeAgent(taskId, { model: constructorModel })

      // Mock the query function to capture the options passed
      let capturedOptions: any
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(({ prompt: _prompt, options }) => {
        capturedOptions = options
        const generator = async function* () {
          const textMessage: SDKMessage = {
            type: 'assistant',
            message: {
              id: 'msg_123',
              type: 'message',
              role: 'assistant',
              model: runOptionsModel, // Should use the model from runOptions
              content: [
                {
                  type: 'text',
                  text: 'Response using model from runOptions.',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
            },
            parent_tool_use_id: null,
            session_id: 'test-session',
          }
          yield textMessage
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      // Run with model in runOptions
      const events: unknown[] = []
      for await (const event of agent.run('test message', { model: runOptionsModel })) {
        events.push(event)
      }

      // Verify the model from runOptions was used
      expect(capturedOptions).toBeDefined()
      expect(capturedOptions.model).toBe(runOptionsModel)
    })

    it('should use model from constructor options when not in runOptions', async () => {
      const taskId = 'test-task-id'
      const constructorModel = 'claude-3-opus-20240229'

      // Create agent with model in constructor options
      const agent = new ClaudeCodeAgent(taskId, { model: constructorModel })

      // Mock the query function to capture the options passed
      let capturedOptions: any
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(({ prompt: _prompt, options }) => {
        capturedOptions = options
        const generator = async function* () {
          const textMessage: SDKMessage = {
            type: 'assistant',
            message: {
              id: 'msg_123',
              type: 'message',
              role: 'assistant',
              model: constructorModel,
              content: [
                {
                  type: 'text',
                  text: 'Response using model from constructor.',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
            },
            parent_tool_use_id: null,
            session_id: 'test-session',
          }
          yield textMessage
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      // Run without model in runOptions
      const events: unknown[] = []
      for await (const event of agent.run('test message', {})) {
        events.push(event)
      }

      // Verify the model from constructor options was used
      expect(capturedOptions).toBeDefined()
      expect(capturedOptions.model).toBe(constructorModel)
    })

    it('should use default model when no model is specified', async () => {
      const taskId = 'test-task-id'
      const defaultModel = 'claude-sonnet-4-20250514'

      // Create agent without model
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function to capture the options passed
      let capturedOptions: any
      const { query } = await import('@anthropic-ai/claude-code')
      vi.mocked(query).mockImplementation(({ prompt: _prompt, options }) => {
        capturedOptions = options
        const generator = async function* () {
          const textMessage: SDKMessage = {
            type: 'assistant',
            message: {
              id: 'msg_123',
              type: 'message',
              role: 'assistant',
              model: defaultModel,
              content: [
                {
                  type: 'text',
                  text: 'Response using default model.',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
            },
            parent_tool_use_id: null,
            session_id: 'test-session',
          }
          yield textMessage
        }
        const gen = generator()
        return {
          ...gen,
          [Symbol.asyncIterator]: () => gen,
          interrupt: vi.fn(),
        }
      })

      // Run without model
      const events: unknown[] = []
      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Verify no model was set (should be undefined when not specified)
      expect(capturedOptions).toBeDefined()
      expect(capturedOptions.model).toBeUndefined()
    })
  })
})

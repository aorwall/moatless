import { describe, expect, it, vi } from 'vitest'

// Mock the @anthropic-ai/claude-agent-sdk module
vi.mock('@anthropic-ai/claude-agent-sdk', async () => {
  const { createMockQueryImplementation } = await import('../../test-utils/claude-agent-mocks.js')
  return {
    query: createMockQueryImplementation(),
  }
})

import {
  createMockQuery,
  createMockSDKAssistantMessage,
} from '../../test-utils/claude-agent-mocks.js'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent.js'

describe('ClaudeCodeAgent - System Prompt E2E', () => {
  describe('appendSystemPrompt parameter', () => {
    it('should pass appendSystemPrompt to Claude SDK with systemPrompt.append', async () => {
      const taskId = 'test-task-system-prompt'
      const agent = new ClaudeCodeAgent(taskId)

      // Mock the query function and capture the parameters
      const { query } = await import('@anthropic-ai/claude-agent-sdk')
      const mockQuery = vi.mocked(query)

      mockQuery.mockImplementation((params) => {
        // Capture the systemPrompt parameter for verification
        return createMockQuery([
          createMockSDKAssistantMessage({
            message: {
              id: 'msg_test',
              type: 'message',
              role: 'assistant',
              model: 'claude-sonnet-4-20250514',
              content: [
                {
                  type: 'text',
                  text: 'Response with custom system prompt',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 100,
                output_tokens: 20,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
              },
            },
          }),
        ])
      })

      const customSystemPrompt = 'You are a specialized assistant for testing'
      const events: unknown[] = []

      for await (const event of agent.run('test message', {
        appendSystemPrompt: customSystemPrompt,
      })) {
        events.push(event)
      }

      // Verify query was called with systemPrompt containing the custom prompt
      expect(mockQuery).toHaveBeenCalled()
      const queryParams = mockQuery.mock.calls[0][0]

      // The appendSystemPrompt should be in systemPrompt.append
      expect(queryParams.systemPrompt).toBeDefined()
      expect(queryParams.systemPrompt.type).toBe('preset')
      expect(queryParams.systemPrompt.preset).toBe('claude_code')
      expect(queryParams.systemPrompt.append).toBe(customSystemPrompt)
    })

    it('should work without appendSystemPrompt when not provided', async () => {
      const taskId = 'test-task-no-system-prompt'
      const agent = new ClaudeCodeAgent(taskId)

      const { query } = await import('@anthropic-ai/claude-agent-sdk')
      const mockQuery = vi.mocked(query)

      mockQuery.mockImplementation(() =>
        createMockQuery([
          createMockSDKAssistantMessage({
            message: {
              id: 'msg_test',
              type: 'message',
              role: 'assistant',
              model: 'claude-sonnet-4-20250514',
              content: [
                {
                  type: 'text',
                  text: 'Response without custom system prompt',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 50,
                output_tokens: 15,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
              },
            },
          }),
        ]),
      )

      const events: unknown[] = []

      for await (const event of agent.run('test message')) {
        events.push(event)
      }

      // Verify query was called
      expect(mockQuery).toHaveBeenCalled()
      const queryParams = mockQuery.mock.calls[0][0]

      // systemPrompt should not be set when appendSystemPrompt not provided
      expect(queryParams.systemPrompt).toBeUndefined()
    })

    it('should handle multipart system prompts', async () => {
      const taskId = 'test-task-multipart'
      const agent = new ClaudeCodeAgent(taskId)

      const { query } = await import('@anthropic-ai/claude-agent-sdk')
      const mockQuery = vi.mocked(query)

      mockQuery.mockImplementation(() =>
        createMockQuery([
          createMockSDKAssistantMessage({
            message: {
              id: 'msg_test',
              type: 'message',
              role: 'assistant',
              model: 'claude-sonnet-4-20250514',
              content: [
                {
                  type: 'text',
                  text: 'Response',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 150,
                output_tokens: 10,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
              },
            },
          }),
        ]),
      )

      const multipartSystemPrompt = `
**Repository Configuration**
You are working in a React + TypeScript project.

**Special Instructions**
- Follow existing patterns
- Use TypeScript strict mode
- Write tests for all new code
`.trim()

      const events: unknown[] = []

      for await (const event of agent.run('Build a new feature', {
        appendSystemPrompt: multipartSystemPrompt,
      })) {
        events.push(event)
      }

      // Verify system prompt contains our custom content
      expect(mockQuery).toHaveBeenCalled()
      const queryParams = mockQuery.mock.calls[0][0]

      expect(queryParams.systemPrompt.append).toContain('**Repository Configuration**')
      expect(queryParams.systemPrompt.append).toContain('**Special Instructions**')
    })

    it('should handle very long system prompts', async () => {
      const taskId = 'test-task-long-prompt'
      const agent = new ClaudeCodeAgent(taskId)

      const { query } = await import('@anthropic-ai/claude-agent-sdk')
      const mockQuery = vi.mocked(query)

      mockQuery.mockImplementation(() =>
        createMockQuery([
          createMockSDKAssistantMessage({
            message: {
              id: 'msg_test',
              type: 'message',
              role: 'assistant',
              model: 'claude-sonnet-4-20250514',
              content: [
                {
                  type: 'text',
                  text: 'Handled long prompt',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 5000,
                output_tokens: 10,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
              },
            },
          }),
        ]),
      )

      // Generate a long system prompt (simulating a comprehensive custom prompt)
      const longSystemPrompt = `
# Comprehensive Development Guidelines

## Architecture
${'- Detailed architectural guideline\n'.repeat(50)}

## Code Quality
${'- Code quality standard\n'.repeat(50)}

## Testing
${'- Testing requirement\n'.repeat(50)}
`.trim()

      const events: unknown[] = []

      for await (const event of agent.run('Complex task', {
        appendSystemPrompt: longSystemPrompt,
      })) {
        events.push(event)
      }

      // Verify the long prompt is included
      expect(mockQuery).toHaveBeenCalled()
      const queryParams = mockQuery.mock.calls[0][0]

      expect(queryParams.systemPrompt.append).toContain('# Comprehensive Development Guidelines')
      expect(queryParams.systemPrompt.append).toContain('## Architecture')
      expect(queryParams.systemPrompt.append).toContain('## Code Quality')
      expect(queryParams.systemPrompt.append).toContain('## Testing')
    })
  })

  describe('System Prompt Integration with Other Options', () => {
    it('should work with appendSystemPrompt alongside other run options', async () => {
      const taskId = 'test-task-combined-options'
      const agent = new ClaudeCodeAgent(taskId)

      const { query } = await import('@anthropic-ai/claude-agent-sdk')
      const mockQuery = vi.mocked(query)

      mockQuery.mockImplementation(() =>
        createMockQuery([
          createMockSDKAssistantMessage({
            message: {
              id: 'msg_test',
              type: 'message',
              role: 'assistant',
              model: 'claude-opus-4-20250514',
              content: [
                {
                  type: 'text',
                  text: 'Response with all options',
                },
              ],
              stop_reason: 'end_turn',
              stop_sequence: null,
              usage: {
                input_tokens: 200,
                output_tokens: 25,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
              },
            },
          }),
        ]),
      )

      const events: unknown[] = []

      for await (const event of agent.run('test message', {
        workspaceDir: '/test/workspace',
        iterationNumber: 5,
        model: 'claude-opus-4-20250514',
        hasPreviewServer: true,
        appendSystemPrompt: 'Custom system prompt with other options',
      })) {
        events.push(event)
      }

      // Verify all options were passed correctly
      expect(mockQuery).toHaveBeenCalled()
      const queryParams = mockQuery.mock.calls[0][0]

      // Verify system prompt
      expect(queryParams.systemPrompt.append).toBe('Custom system prompt with other options')

      // Verify model was set
      expect(queryParams.model).toBe('claude-opus-4-20250514')
    })
  })
})

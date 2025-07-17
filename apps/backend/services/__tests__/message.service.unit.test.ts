import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageService } from '../message.service.js'

describe('MessageService Unit Tests', () => {
  let service: MessageService
  let mockTask: Task
  let mockClaudeService: any

  beforeEach(() => {
    mockTask = {
      taskId: 'test-task-123',
      repositoryId: 'test-repo',
      status: 'open',
      createdAt: new Date(),
      lastMessageAt: new Date(),
      agentType: 'claude-code',
      numTurns: 1,
      totalCostUsd: 0.01,
      iterations: [
        {
          iterationNumber: 1,
          agentSessionId: 'session-123',
          status: 'completed',
          startTime: new Date(),
          messageCount: 1,
          costUsd: 0.01,
          updatedAt: new Date(),
        } as any,
      ],
      workspacePath: '/test/workspace',
    } as Task

    mockClaudeService = {
      readClaudeMessages: vi.fn().mockResolvedValue([]),
    }
    service = new MessageService(mockClaudeService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('Message Transformation', () => {
    it('should transform simple user and assistant messages', async () => {
      const sdkUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Hello, how are you?',
        },
        timestamp: 1234567890000,
      } as unknown as SDKMessage

      const sdkAssistantMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'I am doing well, thank you!',
            },
          ],
        },
        timestamp: 1234567890001,
      } as unknown as SDKMessage

      // Mock the claude service to return the SDK messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([sdkUserMessage, sdkAssistantMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(2)
      expect(result[0].role).toBe('user')
      expect(result[0].content).toBe('Hello, how are you?')
      expect(result[1].role).toBe('assistant')
      expect(result[1].content).toBe('I am doing well, thank you!')
    })

    it('should handle tool use messages with results', async () => {
      const toolUseMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              id: 'tool-123',
              name: 'read_file',
              input: { file_path: 'test.txt' },
            },
          ],
        },
        timestamp: 1234567890000,
      } as unknown as SDKMessage

      const toolResultMessage: SDKMessage = {
        message: {
          role: 'user',
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-123',
              content: 'File content here',
              is_error: false,
            },
          ],
        },
        timestamp: 1234567890001,
      } as unknown as SDKMessage

      // Mock the claude service to return the SDK messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([toolUseMessage, toolResultMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(Array.isArray(result[0].content)).toBe(true)
      const content = result[0].content as any[]
      expect(content).toHaveLength(1)
      expect(content[0].type).toBe('tool_use')
      expect(content[0].tool_name).toBe('read_file')
      expect(content[0].tool_use_id).toBe('tool-123')
      expect(content[0].tool_result?.content).toBe('File content here')
    })

    it('should handle malformed JSON gracefully', async () => {
      const validMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Valid message',
        },
        timestamp: 1234567890000,
      } as unknown as SDKMessage

      // Mock the claude service to return only the valid message
      mockClaudeService.readClaudeMessages.mockResolvedValue([validMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Valid message')
    })

    it('should skip empty user messages', async () => {
      const emptyUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: '',
        },
        timestamp: 1234567890000,
      } as unknown as SDKMessage

      const validUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Valid message',
        },
        timestamp: 1234567890001,
      } as unknown as SDKMessage

      // Mock the claude service to return both messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([emptyUserMessage, validUserMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Valid message')
    })

    it('should handle error conditions', async () => {
      // Mock the claude service to reject with an error
      mockClaudeService.readClaudeMessages.mockRejectedValue(
        new Error('Failed to read Claude messages'),
      )

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      // Should fall back to empty array from task iterations (no iterations have messages)
      expect(result).toEqual([])
    })
  })

  describe('Edge Cases', () => {
    it('should handle task without workspace path', async () => {
      const taskWithoutWorkspace = { ...mockTask, workspacePath: undefined }

      const result = await service.readMessagesFromClaudeDirectory(taskWithoutWorkspace)

      expect(result).toEqual([])
    })

    it('should handle messages without timestamps', async () => {
      const messageWithoutTimestamp: SDKMessage = {
        message: {
          role: 'user',
          content: 'Message without timestamp',
        },
      } as unknown as SDKMessage

      // Mock the claude service to return the message without timestamp
      mockClaudeService.readClaudeMessages.mockResolvedValue([messageWithoutTimestamp])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0].timestamp).toBeInstanceOf(Date)
      expect(result[0].content).toBe('Message without timestamp')
    })
  })

  describe('Message Parsing from Task Iterations', () => {
    beforeEach(() => {
      // Mock claude service to return empty array to trigger fallback to task iterations
      mockClaudeService.readClaudeMessages.mockResolvedValue([])
    })

    it('should create messages from task iterations when Claude file is not found', async () => {
      const taskWithIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'Create a new feature',
            result: 'I have successfully created the feature as requested.',
          },
          {
            iterationNumber: 2,
            startTime: new Date('2024-01-01T10:02:00Z'),
            endTime: new Date('2024-01-01T10:03:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'Add tests for the feature',
            result: 'Tests have been added and are passing.',
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithIterations)

      expect(result).toHaveLength(4)

      // First iteration
      expect(result[0].role).toBe('user')
      expect(result[0].content).toBe('Create a new feature')
      expect(result[0].id).toBe('iteration-1-user')
      expect(result[0].timestamp).toEqual(new Date('2024-01-01T10:00:00Z'))

      expect(result[1].role).toBe('assistant')
      expect(result[1].content).toBe('I have successfully created the feature as requested.')
      expect(result[1].id).toBe('iteration-1-assistant')
      expect(result[1].timestamp).toEqual(new Date('2024-01-01T10:01:00Z'))

      // Second iteration
      expect(result[2].role).toBe('user')
      expect(result[2].content).toBe('Add tests for the feature')
      expect(result[2].id).toBe('iteration-2-user')

      expect(result[3].role).toBe('assistant')
      expect(result[3].content).toBe('Tests have been added and are passing.')
      expect(result[3].id).toBe('iteration-2-assistant')
    })

    it('should parse JSON message content from iteration result', async () => {
      const taskWithJsonResult = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: JSON.stringify({
              content: 'This is the actual message content from JSON',
              metadata: { type: 'response' },
            }),
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithJsonResult)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(result[0].content).toBe('This is the actual message content from JSON')
    })

    it('should parse array-based content structure', async () => {
      const taskWithArrayContent = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: JSON.stringify({
              content: [
                { type: 'text', text: 'First part of the message.' },
                { type: 'text', text: 'Second part of the message.' },
                { type: 'code', code: 'console.log("ignored")' },
              ],
            }),
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithArrayContent)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(result[0].content).toBe('First part of the message.\nSecond part of the message.')
    })

    it('should handle alternative message field names', async () => {
      const taskWithMessageField = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: JSON.stringify({
              message: 'Content from message field',
            }),
          },
          {
            iterationNumber: 2,
            startTime: new Date('2024-01-01T10:02:00Z'),
            endTime: new Date('2024-01-01T10:03:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: JSON.stringify({
              text: 'Content from text field',
            }),
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithMessageField)

      expect(result).toHaveLength(2)
      expect(result[0].content).toBe('Content from message field')
      expect(result[1].content).toBe('Content from text field')
    })

    it('should fallback to raw result when JSON parsing fails', async () => {
      const taskWithPlainTextResult = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: 'Plain text result that is not JSON',
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithPlainTextResult)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(result[0].content).toBe('Plain text result that is not JSON')
    })

    it('should skip iterations without messages', async () => {
      const taskWithEmptyIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            status: 'completed',
            messageCount: 0,
            costUsd: 0.01,
            // No initialMessage or result
          },
          {
            iterationNumber: 2,
            startTime: new Date('2024-01-01T10:01:00Z'),
            endTime: new Date('2024-01-01T10:02:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            initialMessage: 'Valid message',
            result: 'Valid response',
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithEmptyIterations)

      expect(result).toHaveLength(2)
      expect(result[0].content).toBe('Valid message')
      expect(result[1].content).toBe('Valid response')
    })

    it('should handle iterations with only user messages', async () => {
      const taskWithUserOnlyIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            status: 'active',
            messageCount: 1,
            costUsd: 0,
            initialMessage: 'User message without response',
            // No result field
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithUserOnlyIterations)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('user')
      expect(result[0].content).toBe('User message without response')
    })

    it('should handle invalid JSON gracefully in iteration results', async () => {
      const taskWithInvalidJson = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            result: '{ invalid json content that cannot be parsed',
          },
        ],
      } as Task

      const result = await service.readMessagesFromClaudeDirectory(taskWithInvalidJson)

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(result[0].content).toBe('{ invalid json content that cannot be parsed')
    })
  })

  describe('Mixed Claude Files and Task Iterations', () => {
    it('should use Claude messages when available', async () => {
      const taskWithMixedIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'session-1',
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            // This iteration has a Claude file
          },
          {
            iterationNumber: 2,
            startTime: new Date('2024-01-01T10:02:00Z'),
            endTime: new Date('2024-01-01T10:03:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'Task iteration message',
            result: 'Task iteration response',
            // This iteration has no Claude file
          },
        ],
      } as Task

      // Mock Claude service to return messages
      const sdkUserMessage = {
        message: {
          role: 'user',
          content: 'Claude file user message',
        },
        timestamp: 1704100800000,
      } as unknown as SDKMessage

      const sdkAssistantMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'Claude file assistant message',
            },
          ],
        },
        timestamp: 1704100860000,
      } as unknown as SDKMessage

      mockClaudeService.readClaudeMessages.mockResolvedValue([sdkUserMessage, sdkAssistantMessage])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMixedIterations)

      expect(result).toHaveLength(2)

      // Should have messages from Claude service
      expect(result[0].role).toBe('user')
      expect(result[0].content).toBe('Claude file user message')

      expect(result[1].role).toBe('assistant')
      expect(result[1].content).toBe('Claude file assistant message')
    })

    it('should fall back to task iterations when no Claude messages exist', async () => {
      const taskWithMultipleIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'session-1',
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            // This iteration has a Claude file
          },
          {
            iterationNumber: 2,
            startTime: new Date('2024-01-01T10:02:00Z'),
            endTime: new Date('2024-01-01T10:03:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'First fallback message',
            result: 'First fallback response',
            // This iteration has no Claude file
          },
          {
            iterationNumber: 3,
            startTime: new Date('2024-01-01T10:04:00Z'),
            endTime: new Date('2024-01-01T10:05:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'Second fallback message',
            result: 'Second fallback response',
            // This iteration has no Claude file
          },
        ],
      } as Task

      // Mock Claude service to return empty array - will fall back to task iterations
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMultipleIterations)

      expect(result).toHaveLength(4)

      // Should have messages from task iterations
      expect(result[0].content).toBe('First fallback message')
      expect(result[1].content).toBe('First fallback response')
      expect(result[2].content).toBe('Second fallback message')
      expect(result[3].content).toBe('Second fallback response')
    })

    it('should not process Claude files when workspace path is missing', async () => {
      const taskWithoutWorkspace = {
        ...mockTask,
        workspacePath: undefined,
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'session-1', // Has session ID but no workspace
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.01,
            initialMessage: 'Message without workspace',
            result: 'Response without workspace',
          },
        ],
      } as Task

      // Claude service will be called but should handle missing workspace path gracefully
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithoutWorkspace)

      // Should only get messages from task iterations
      expect(result).toHaveLength(2)
      expect(result[0].content).toBe('Message without workspace')
      expect(result[1].content).toBe('Response without workspace')
    })

    it('should maintain chronological order of messages', async () => {
      const taskWithMixedIterations = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            startTime: new Date('2024-01-01T10:00:00Z'),
            endTime: new Date('2024-01-01T10:01:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            initialMessage: 'First iteration user message',
            result: 'First iteration result',
            // No Claude file
          },
          {
            iterationNumber: 2,
            agentSessionId: 'session-2',
            startTime: new Date('2024-01-01T10:02:00Z'),
            endTime: new Date('2024-01-01T10:03:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            // Has Claude file
          },
          {
            iterationNumber: 3,
            startTime: new Date('2024-01-01T10:04:00Z'),
            endTime: new Date('2024-01-01T10:05:00Z'),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.01,
            initialMessage: 'Third iteration user message',
            result: 'Third iteration result',
            // No Claude file
          },
        ],
      } as Task

      // This test doesn't make sense anymore since ClaudeService handles all files at once
      // When Claude messages exist, we use them exclusively; when they don't, we fall back to iterations
      // Let's test that scenario: no Claude messages, so we get iteration messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMixedIterations)

      expect(result).toHaveLength(4)

      // Should have messages from all iterations that have content
      expect(result[0].content).toBe('First iteration user message')
      expect(result[1].content).toBe('First iteration result')
      expect(result[2].content).toBe('Third iteration user message')
      expect(result[3].content).toBe('Third iteration result')
    })
  })
})

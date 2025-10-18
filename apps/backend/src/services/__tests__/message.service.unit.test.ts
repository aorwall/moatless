import type { AgentMessage, TaskResponse } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockStorageBackend } from '../../test/mocks/services.mock.js'
import { MessageService } from '../message.service.js'

describe('MessageService Unit Tests', () => {
  let service: MessageService
  let mockTask: TaskResponse
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
          status: 'completed',
          startTime: new Date(),
          messageCount: 1,
          costUsd: 0.01,
          agentSessionId: 'mock-session-id', // Add this so iterations get processed
        },
      ],
      workspacePath: '/test/workspace',
    }

    mockClaudeService = {
      readClaudeMessages: vi.fn().mockResolvedValue([]),
    }

    const mockStorage = createMockStorageBackend()

    const mockUploadService = {
      uploadFile: vi.fn(),
      getFile: vi.fn(),
      getFileMetadata: vi.fn(),
      deleteFile: vi.fn(),
    } as any

    service = new MessageService(mockClaudeService, mockStorage, mockUploadService)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('Message Transformation', () => {
    it('should transform simple user and assistant messages', async () => {
      const agentUserMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'Hello, how are you?',
        },
        uuid: 'user-msg-123',
        timestamp: '2024-01-01T10:00:00.000Z',
      }

      const agentAssistantMessage: AgentMessage = {
        parentUuid: 'user-msg-123',
        isSidechain: false,
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'assistant',
        message: {
          id: 'msg-123',
          type: 'message',
          role: 'assistant',
          model: 'claude-3-opus',
          content: [
            {
              type: 'text',
              text: 'I am doing well, thank you!',
            },
          ],
        },
        uuid: 'assistant-msg-123',
        timestamp: '2024-01-01T10:00:01.000Z',
      }

      // Mock the claude service to return the agent messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([
        agentUserMessage,
        agentAssistantMessage,
      ])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(2)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('Hello, how are you?')
      expect(result[0]?.id).toBe('user-msg-123')
      expect(result[1]?.role).toBe('assistant')
      expect(result[1]?.content).toBe('I am doing well, thank you!')
      expect(result[1]?.id).toBe('assistant-msg-123')
    })

    it('should handle tool use messages with results', async () => {
      const toolUseMessage: AgentMessage = {
        parentUuid: 'user-msg-123',
        isSidechain: false,
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'assistant',
        message: {
          id: 'msg-456',
          type: 'message',
          role: 'assistant',
          model: 'claude-3-opus',
          content: [
            {
              type: 'tool_use',
              id: 'tool-123',
              name: 'read_file',
              input: { file_path: 'test.txt' },
            },
          ],
        },
        uuid: 'tool-use-msg-123',
        timestamp: '2024-01-01T10:00:02.000Z',
      }

      const toolResultMessage: AgentMessage = {
        parentUuid: 'tool-use-msg-123',
        isSidechain: false,
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
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
        uuid: 'tool-result-msg-123',
        timestamp: '2024-01-01T10:00:03.000Z',
      }

      // Mock the claude service to return the agent messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([toolUseMessage, toolResultMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(Array.isArray(result[0]?.content)).toBe(true)
      const content = result[0]?.content as any[]
      expect(content).toHaveLength(1)
      expect(content[0].type).toBe('tool_use')
      expect(content[0].tool_name).toBe('read_file')
      expect(content[0].tool_use_id).toBe('tool-123')
      expect(content[0].tool_result?.content).toBe('File content here')
    })

    it('should handle malformed JSON gracefully', async () => {
      const validMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'Valid message',
        },
        uuid: 'valid-msg-123',
        timestamp: '2024-01-01T10:00:00.000Z',
      }

      // Mock the claude service to return only the valid message
      mockClaudeService.readClaudeMessages.mockResolvedValue([validMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.content).toBe('Valid message')
    })

    it('should skip empty user messages', async () => {
      const emptyUserMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: '',
        },
        uuid: 'empty-msg-123',
        timestamp: '2024-01-01T10:00:00.000Z',
      }

      const validUserMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'Valid message',
        },
        uuid: 'valid-msg-456',
        timestamp: '2024-01-01T10:00:01.000Z',
      }

      // Mock the claude service to return both messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([emptyUserMessage, validUserMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.content).toBe('Valid message')
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
      const messageWithTimestamp: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'Message with timestamp',
        },
        uuid: 'msg-no-ts-123',
        timestamp: '2024-01-01T10:00:00.000Z',
      }

      // Mock the claude service to return the message
      mockClaudeService.readClaudeMessages.mockResolvedValue([messageWithTimestamp])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.timestamp).toBeInstanceOf(Date)
      expect(result[0]?.content).toBe('Message with timestamp')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithIterations)

      expect(result).toHaveLength(4)

      // First iteration
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('Create a new feature')
      expect(result[0]?.id).toBe('iteration-1-user')
      expect(result[0]?.timestamp).toEqual(new Date('2024-01-01T10:00:00Z'))

      expect(result[1]?.role).toBe('assistant')
      expect(result[1]?.content).toBe('I have successfully created the feature as requested.')
      expect(result[1]?.id).toBe('iteration-1-assistant')
      expect(result[1]?.timestamp).toEqual(new Date('2024-01-01T10:01:00Z'))

      // Second iteration
      expect(result[2]?.role).toBe('user')
      expect(result[2]?.content).toBe('Add tests for the feature')
      expect(result[2]?.id).toBe('iteration-2-user')

      expect(result[3]?.role).toBe('assistant')
      expect(result[3]?.content).toBe('Tests have been added and are passing.')
      expect(result[3]?.id).toBe('iteration-2-assistant')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithJsonResult)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toBe('This is the actual message content from JSON')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithArrayContent)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toBe('First part of the message.\nSecond part of the message.')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithMessageField)

      expect(result).toHaveLength(2)
      expect(result[0]?.content).toBe('Content from message field')
      expect(result[1]?.content).toBe('Content from text field')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithPlainTextResult)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toBe('Plain text result that is not JSON')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithEmptyIterations)

      expect(result).toHaveLength(2)
      expect(result[0]?.content).toBe('Valid message')
      expect(result[1]?.content).toBe('Valid response')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithUserOnlyIterations)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('User message without response')
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
      } as TaskResponse

      const result = await service.readMessagesFromClaudeDirectory(taskWithInvalidJson)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toBe('{ invalid json content that cannot be parsed')
    })
  })

  describe('Real-world AgentMessage Examples', () => {
    it('should parse user message with commit request', async () => {
      const userMessage: AgentMessage = {
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'commit changes',
        },
        uuid: 'b87215e7-8274-4a86-84c0-01998dd143cd',
        timestamp: '2025-06-27T09:05:45.614Z',
      }

      mockClaudeService.readClaudeMessages.mockResolvedValue([userMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('commit changes')
      expect(result[0]?.id).toBe('b87215e7-8274-4a86-84c0-01998dd143cd')
      expect(result[0]?.timestamp).toEqual(new Date('2025-06-27T09:05:45.614Z'))
    })

    it('should parse assistant message with tool use', async () => {
      const assistantMessage: AgentMessage = {
        parentUuid: 'e127ad75-ca76-4278-9a44-9df0212bdc64',
        isSidechain: false,
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        message: {
          id: 'msg_01USKTt6JCUs8NSKExGdCHTv',
          type: 'message',
          role: 'assistant',
          model: 'claude-opus-4-20250514',
          content: [
            {
              type: 'tool_use',
              id: 'toolu_011o12pQ7EEhYmKNCeawnPKN',
              name: 'Bash',
              input: {
                command: 'git status',
                description: 'Check current git status',
              },
            },
          ],
          stop_reason: null,
          stop_sequence: null,
          usage: {
            input_tokens: 4,
            cache_creation_input_tokens: 8404,
            cache_read_input_tokens: 10164,
            output_tokens: 3,
            service_tier: 'standard',
          },
        },
        requestId: 'req_011CQYTbqesGwzdZ2KmpeKve',
        type: 'assistant',
        uuid: 'd17a3e06-4bae-4105-b260-26e2594d6364',
        timestamp: '2025-06-27T09:05:52.318Z',
      }

      mockClaudeService.readClaudeMessages.mockResolvedValue([assistantMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      const content = result[0]?.content as any[]
      expect(content[0].type).toBe('tool_use')
      expect(content[0].tool_name).toBe('Bash')
      expect(content[0].tool_use_id).toBe('toolu_011o12pQ7EEhYmKNCeawnPKN')
    })

    it('should parse error message from assistant', async () => {
      const errorMessage: AgentMessage = {
        parentUuid: 'fd64c7c9-a911-4c03-8d3f-f9336ae81cab',
        isSidechain: false,
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: 'f2489aab-3628-4eb6-b826-09defec35152',
        version: '1.0.55',
        gitBranch: 'workspace-test-task-1752855167445',
        type: 'assistant',
        message: {
          id: '338afb05-c1a6-496e-ab9b-19be38ad8078',
          model: '<synthetic>',
          role: 'assistant',
          stop_reason: 'stop_sequence',
          stop_sequence: '',
          type: 'message',
          usage: {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            server_tool_use: {
              web_search_requests: 0,
            },
            service_tier: null,
          },
          content: [
            {
              type: 'text',
              text: 'API Error: 401 {"type":"error","error":{"type":"authentication_error","message":"OAuth token has expired. Please obtain a new token or refresh your existing token."}}',
            },
          ],
        },
        uuid: '8c7eaeba-42a5-45cb-b39c-b7c9b8fe5769',
        timestamp: '2025-07-18T16:15:25.140Z',
      }

      mockClaudeService.readClaudeMessages.mockResolvedValue([errorMessage])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toContain('API Error: 401')
      expect(result[0]?.content).toContain('OAuth token has expired')
    })

    it('should handle tool result with output', async () => {
      const toolResultMsg: AgentMessage = {
        parentUuid: '00d372ae-09a6-441a-8fa3-42e01c952845',
        isSidechain: false,
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: [
            {
              tool_use_id: 'toolu_011t94XWe5afDcEVuhwF5u4q',
              type: 'tool_result',
              content:
                "6c89efa Merge branch 'workspace-77e52a31-2cc7-45d3-9574-e79744e000ff' into main\n289b655 fix: remove redundant running indicator Badge from chat header",
              is_error: false,
            },
          ],
        },
        uuid: '4d299b0b-89f6-48be-bd57-61c06ad92f80',
        timestamp: '2025-06-27T09:05:54.422Z',
        toolUseResult: {
          stdout:
            "6c89efa Merge branch 'workspace-77e52a31-2cc7-45d3-9574-e79744e000ff' into main\n289b655 fix: remove redundant running indicator Badge from chat header",
          stderr: '',
          interrupted: false,
          isImage: false,
        },
      }

      // This is typically paired with a tool use message
      const toolUseMsg: AgentMessage = {
        parentUuid: 'parent-123',
        isSidechain: false,
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        type: 'assistant',
        message: {
          id: 'msg-123',
          type: 'message',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              id: 'toolu_011t94XWe5afDcEVuhwF5u4q',
              name: 'git',
              input: { command: 'git log' },
            },
          ],
        },
        uuid: 'tool-use-123',
        timestamp: '2025-06-27T09:05:53.000Z',
      }

      mockClaudeService.readClaudeMessages.mockResolvedValue([toolUseMsg, toolResultMsg])

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      const content = result[0]?.content as any[]
      expect(content[0].tool_result?.content).toContain('6c89efa Merge branch')
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
      } as TaskResponse

      // Mock Claude service to return messages
      const agentUserMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-1',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: 'Claude file user message',
        },
        uuid: 'user-claude-123',
        timestamp: '2024-01-01T10:00:00.000Z',
      }

      const agentAssistantMessage: AgentMessage = {
        parentUuid: 'user-claude-123',
        isSidechain: false,
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-1',
        version: '1.0.35',
        type: 'assistant',
        message: {
          id: 'msg-claude-456',
          type: 'message',
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'Claude file assistant message',
            },
          ],
        },
        uuid: 'assistant-claude-456',
        timestamp: '2024-01-01T10:01:00.000Z',
      }

      mockClaudeService.readClaudeMessages.mockResolvedValue([
        agentUserMessage,
        agentAssistantMessage,
      ])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMixedIterations)

      expect(result).toHaveLength(2)

      // Should have messages from Claude service
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('Claude file user message')

      expect(result[1]?.role).toBe('assistant')
      expect(result[1]?.content).toBe('Claude file assistant message')
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
      } as TaskResponse

      // Mock Claude service to return empty array - will fall back to task iterations
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMultipleIterations)

      expect(result).toHaveLength(4)

      // Should have messages from task iterations
      expect(result[0]?.content).toBe('First fallback message')
      expect(result[1]?.content).toBe('First fallback response')
      expect(result[2]?.content).toBe('Second fallback message')
      expect(result[3]?.content).toBe('Second fallback response')
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
      } as TaskResponse

      // Claude service will be called but should handle missing workspace path gracefully
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithoutWorkspace)

      // Should only get messages from task iterations
      expect(result).toHaveLength(2)
      expect(result[0]?.content).toBe('Message without workspace')
      expect(result[1]?.content).toBe('Response without workspace')
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
      } as TaskResponse

      // This test doesn't make sense anymore since ClaudeService handles all files at once
      // When Claude messages exist, we use them exclusively; when they don't, we fall back to iterations
      // Let's test that scenario: no Claude messages, so we get iteration messages
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithMixedIterations)

      expect(result).toHaveLength(4)

      // Should have messages from all iterations that have content
      expect(result[0]?.content).toBe('First iteration user message')
      expect(result[1]?.content).toBe('First iteration result')
      expect(result[2]?.content).toBe('Third iteration user message')
      expect(result[3]?.content).toBe('Third iteration result')
    })
  })

  describe('Initial Message After Task Creation', () => {
    it('should return only initial message for newly created task with no Claude messages', async () => {
      const initialMessage = 'Help me build a feature'
      const taskWithInitialMessage = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            status: 'created' as const,
            startTime: new Date('2024-01-01T10:00:00Z'),
            messageCount: 1,
            costUsd: 0,
            initialMessage,
            agentSessionId: 'session-123',
            // No result yet - agent hasn't run
          },
        ],
      } as TaskResponse

      // No Claude messages yet (fresh task)
      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithInitialMessage)

      // Should have exactly one message - the initial user message
      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe(initialMessage)
      expect(result[0]?.id).toBeDefined() // ID can vary based on path taken
      expect(result[0]?.timestamp).toEqual(new Date('2024-01-01T10:00:00Z'))
    })

    it('should return only initial message when Claude messages exist but no user message found', async () => {
      const initialMessage = 'Create a React component'
      const taskWithInitialMessageAndClaudeMessages = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            status: 'active' as const,
            startTime: new Date('2024-01-01T10:00:00Z'),
            messageCount: 1,
            costUsd: 0,
            initialMessage,
            agentSessionId: 'session-456',
            // Agent is starting but no result yet
          },
        ],
      } as TaskResponse

      // Claude messages exist but they don't contain the initial user message
      // (maybe only tool results or system messages)
      const claudeMessages: AgentMessage[] = [
        {
          userType: 'external',
          cwd: '/test/workspace',
          sessionId: 'session-456',
          version: '1.0.35',
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-123',
                content: 'Some tool result',
                is_error: false,
              },
            ],
          },
          uuid: 'tool-result-msg',
          timestamp: '2024-01-01T10:00:01.000Z',
        },
      ]

      mockClaudeService.readClaudeMessages.mockResolvedValue(claudeMessages)

      const result = await service.readMessagesFromClaudeDirectory(
        taskWithInitialMessageAndClaudeMessages,
      )

      // With the new logic, the first user message gets its content replaced with the initial message
      // even if it was originally a tool result, ensuring we always have the correct initial message
      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe(initialMessage) // Content should be replaced with initial message
      expect(result[0]?.id).toBe('tool-result-msg')
      expect(result[0]?.timestamp).toEqual(new Date('2024-01-01T10:00:00Z')) // Timestamp should be updated
    })

    it('should not duplicate initial message when Claude messages already contain user message', async () => {
      const initialMessage = 'Write a function'
      const claudeUserMessage = 'Write a function' // Same content as initial message
      const taskWithBothMessages = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            status: 'active' as const,
            startTime: new Date('2024-01-01T10:00:00Z'),
            messageCount: 1,
            costUsd: 0,
            initialMessage,
            agentSessionId: 'session-789',
          },
        ],
      } as TaskResponse

      // Claude messages already contain the user message
      const claudeMessages: AgentMessage[] = [
        {
          userType: 'external',
          cwd: '/test/workspace',
          sessionId: 'session-789',
          version: '1.0.35',
          type: 'user',
          message: {
            role: 'user',
            content: claudeUserMessage,
          },
          uuid: 'claude-user-msg',
          timestamp: '2024-01-01T10:00:00.000Z',
        },
      ]

      mockClaudeService.readClaudeMessages.mockResolvedValue(claudeMessages)

      const result = await service.readMessagesFromClaudeDirectory(taskWithBothMessages)

      // Should have only one message with content replaced by initial message
      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe(initialMessage) // Content should be replaced
      expect(result[0]?.id).toBe('claude-user-msg')
      expect(result[0]?.timestamp).toEqual(new Date('2024-01-01T10:00:00Z')) // Timestamp should be updated
    })

    it('should replace first user message content with iteration initial message when different', async () => {
      const initialMessage = 'Create a new feature'
      const claudeUserMessage = 'Different message from Claude' // Different content
      const taskWithDifferentMessages = {
        ...mockTask,
        iterations: [
          {
            iterationNumber: 1,
            status: 'active' as const,
            startTime: new Date('2024-01-01T10:00:00Z'),
            messageCount: 1,
            costUsd: 0,
            initialMessage,
            agentSessionId: 'session-different',
          },
        ],
      } as TaskResponse

      // Claude messages contain the user message but with different content
      const claudeMessages: AgentMessage[] = [
        {
          userType: 'external',
          cwd: '/test/workspace',
          sessionId: 'session-different',
          version: '1.0.35',
          type: 'user',
          message: {
            role: 'user',
            content: claudeUserMessage,
          },
          uuid: 'claude-user-msg-different',
          timestamp: '2024-01-01T10:00:01.000Z',
        },
      ]

      mockClaudeService.readClaudeMessages.mockResolvedValue(claudeMessages)

      const result = await service.readMessagesFromClaudeDirectory(taskWithDifferentMessages)

      // Should have only one message with the content replaced by initial message
      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe(initialMessage) // Content should be replaced
      expect(result[0]?.id).toBe('claude-user-msg-different') // ID should remain the same
      expect(result[0]?.timestamp).toEqual(new Date('2024-01-01T10:00:00Z')) // Timestamp should be updated to iteration start time
    })

    it('should handle empty task iterations gracefully', async () => {
      const taskWithNoIterations = {
        ...mockTask,
        iterations: [],
      } as TaskResponse

      mockClaudeService.readClaudeMessages.mockResolvedValue([])

      const result = await service.readMessagesFromClaudeDirectory(taskWithNoIterations)

      // Should return empty array for task with no iterations
      expect(result).toHaveLength(0)
    })
  })
})

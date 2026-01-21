import type { AgentMessage, Task as TaskType } from '@moatless/types'
import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Alias for compatibility with existing tests
type TaskResponse = TaskType
type Task = TaskType

import { EventService } from '../../../events/services/event.service'
import { FileStorageService } from '../../../file-storage'
import { TaskRepository } from '../../../tasks/db/task.repository'
import { TaskMessageRepository } from '../../../tasks/db/task-message.repository'
import { MessageService } from '../message.service'

describe('MessageService Unit Tests', () => {
  let container: Container
  let service: MessageService
  let mockTask: Task
  let mockMessageRepository: any
  let mockTaskRepository: any
  let mockUploadService: any
  let mockEventService: any

  beforeEach(() => {
    container = new Container()

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
          agentSessionId: 'mock-session-id',
        },
      ],
      visibility: 'private' as const,
    }

    mockMessageRepository = {
      getMessages: vi.fn().mockResolvedValue([]),
      append: vi.fn().mockResolvedValue(undefined),
      appendMany: vi.fn().mockResolvedValue(undefined),
      deleteByTask: vi.fn().mockResolvedValue(undefined),
      hasMessages: vi.fn().mockResolvedValue(true),
    }

    mockTaskRepository = {
      get: vi.fn().mockResolvedValue(mockTask),
      getInternal: vi.fn().mockResolvedValue(mockTask),
      save: vi.fn().mockResolvedValue(undefined),
      delete: vi.fn().mockResolvedValue(undefined),
      exists: vi.fn().mockResolvedValue(true),
      listTaskKeys: vi.fn().mockResolvedValue([]),
    }

    mockUploadService = {
      uploadFile: vi.fn(),
      getFile: vi.fn(),
      getFileMetadata: vi.fn(),
      deleteFile: vi.fn(),
    }

    mockEventService = {
      subscribeAll: vi.fn().mockReturnValue(() => {
        /* unsubscribe noop */
      }),
      emitEvent: vi.fn(),
    }

    container.bind(TaskMessageRepository).toConstantValue(mockMessageRepository)
    container.bind(TaskRepository).toConstantValue(mockTaskRepository)
    container.bind(FileStorageService).toConstantValue(mockUploadService)
    container.bind(EventService).toConstantValue(mockEventService)
    container.bind(MessageService).toSelf()

    service = container.get(MessageService)
  })

  afterEach(() => {
    container.unbindAll()
    vi.clearAllMocks()
  })

  describe('getTaskMessages', () => {
    it('should return messages from repository when available', async () => {
      const agentUserMessage: AgentMessage = {
        userType: 'external',
        cwd: '/test/workspace',
        sessionId: 'session-123',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: [{ type: 'text', text: 'Hello, how are you?' }],
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

      mockMessageRepository.getMessages.mockResolvedValue([agentUserMessage, agentAssistantMessage])

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(2)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('Hello, how are you?')
      expect(result[1]?.role).toBe('assistant')
      expect(Array.isArray(result[1]?.content)).toBe(true)
      if (Array.isArray(result[1]?.content)) {
        expect(result[1]?.content[0]?.type).toBe('text')
        expect(result[1]?.content[0]?.text).toBe('I am doing well, thank you!')
      }
    })

    it('should fall back to task iterations when repository is empty', async () => {
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
        ],
      } as TaskResponse

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithIterations)

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(2)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('Create a new feature')
      expect(result[1]?.role).toBe('assistant')
      expect(result[1]?.content).toBe('I have successfully created the feature as requested.')
    })

    it('should handle tool use messages with results from repository', async () => {
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
        type: 'system',
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

      mockMessageRepository.getMessages.mockResolvedValue([toolUseMessage, toolResultMessage])

      const result = await service.getTaskMessages('test-task-123')

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
  })

  describe('Message Parsing from Task Iterations', () => {
    it('should create messages from task iterations when repository is empty', async () => {
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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithIterations)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithJsonResult)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithArrayContent)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithMessageField)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithPlainTextResult)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithEmptyIterations)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithUserOnlyIterations)

      const result = await service.getTaskMessages('test-task-123')

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

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithInvalidJson)

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(result[0]?.content).toBe('{ invalid json content that cannot be parsed')
    })

    it('should handle empty task iterations gracefully', async () => {
      const taskWithNoIterations = {
        ...mockTask,
        iterations: [],
      } as TaskResponse

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(taskWithNoIterations)

      const result = await service.getTaskMessages('test-task-123')

      // Should return empty array for task with no iterations
      expect(result).toHaveLength(0)
    })
  })

  describe('Edge Cases', () => {
    it('should throw error for non-claude-code tasks', async () => {
      const nonClaudeTask = {
        ...mockTask,
        agentType: 'other-agent',
      } as TaskResponse

      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(nonClaudeTask)

      await expect(service.getTaskMessages('test-task-123')).rejects.toThrow(
        'Only Claude Code tasks are supported for message reading',
      )
    })

    it('should throw error when task not found', async () => {
      mockMessageRepository.getMessages.mockResolvedValue([])
      mockTaskRepository.getInternal.mockResolvedValue(null)

      await expect(service.getTaskMessages('non-existent-task')).rejects.toThrow(
        'Task non-existent-task not found',
      )
    })
  })

  describe('Real-world AgentMessage Examples', () => {
    it('should parse user message with commit request from repository', async () => {
      const userMessage: AgentMessage = {
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        type: 'user',
        message: {
          role: 'user',
          content: [{ type: 'text', text: 'commit changes' }],
        },
        uuid: 'b87215e7-8274-4a86-84c0-01998dd143cd',
        timestamp: '2025-06-27T09:05:45.614Z',
      }

      mockMessageRepository.getMessages.mockResolvedValue([userMessage])

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('user')
      expect(result[0]?.content).toBe('commit changes')
      expect(result[0]?.id).toBe('b87215e7-8274-4a86-84c0-01998dd143cd')
      expect(result[0]?.timestamp).toEqual(new Date('2025-06-27T09:05:45.614Z'))
    })

    it('should parse assistant message with tool use from repository', async () => {
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
          model: 'claude-sonnet-4-5-20250929',
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

      mockMessageRepository.getMessages.mockResolvedValue([assistantMessage])

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      const content = result[0]?.content as any[]
      expect(content[0].type).toBe('tool_use')
      expect(content[0].tool_name).toBe('Bash')
      expect(content[0].tool_use_id).toBe('toolu_011o12pQ7EEhYmKNCeawnPKN')
    })

    it('should parse error message from assistant from repository', async () => {
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

      mockMessageRepository.getMessages.mockResolvedValue([errorMessage])

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      expect(Array.isArray(result[0]?.content)).toBe(true)
      if (Array.isArray(result[0]?.content)) {
        const firstContent = result[0]?.content[0]
        expect(firstContent?.type).toBe('text')
        expect(firstContent?.text).toContain('API Error: 401')
        expect(firstContent?.text).toContain('OAuth token has expired')
      }
    })

    it('should handle tool result with output from repository', async () => {
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

      const toolResultMsg: AgentMessage = {
        parentUuid: '00d372ae-09a6-441a-8fa3-42e01c952845',
        isSidechain: false,
        userType: 'external',
        cwd: '/Users/albert/repos/moatless/moatless-swarm-ui',
        sessionId: '0ef34def-db9a-4a85-9586-46df4167dce4',
        version: '1.0.35',
        type: 'system',
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

      mockMessageRepository.getMessages.mockResolvedValue([toolUseMsg, toolResultMsg])

      const result = await service.getTaskMessages('test-task-123')

      expect(result).toHaveLength(1)
      expect(result[0]?.role).toBe('assistant')
      const content = result[0]?.content as any[]
      expect(content[0].tool_result?.content).toContain('6c89efa Merge branch')
    })
  })
})

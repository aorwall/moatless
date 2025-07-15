import { promises as fs } from 'node:fs'
import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { ClaudeCodeSettings, Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageService, createMessageService } from '../message.service.js'
import * as claudeCodeEvents from '../utils/claude-code-events.js'

// Mock the file system
vi.mock('node:fs', () => ({
  promises: {
    readFile: vi.fn(),
  },
}))

// Mock the claude-code-events utility
vi.mock('../utils/claude-code-events.js', () => ({
  getClaudeEventsFilePath: vi.fn(),
}))

describe('MessageService', () => {
  let service: MessageService
  let mockClaudeCodeSettings: ClaudeCodeSettings
  let mockTask: Task
  let mockFs: any
  let mockGetClaudeEventsFilePath: any

  beforeEach(() => {
    mockClaudeCodeSettings = {
      anthropicApiKey: 'test-key',
      model: 'claude-3-haiku-20240307',
      historyFile: '/test/history.jsonl',
      maxTokens: 4000,
      temperature: 0.7,
      enableContextual: true,
      contextualTokenLimit: 2000,
      contextualSampleSize: 10,
      claudeConfigPath: '/test/claude-config',
    }

    mockTask = {
      taskId: 'test-task-123',
      title: 'Test Task',
      description: 'Test task description',
      repositoryId: 'test-repo',
      status: 'created',
      iterations: [
        {
          iterationNumber: 1,
          agentSessionId: 'session-123',
          status: 'completed',
          message: 'Test message',
          createdAt: new Date(),
          updatedAt: new Date(),
        },
      ],
      workspaceId: 'test-workspace',
      workspacePath: '/test/workspace',
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    service = new MessageService(mockClaudeCodeSettings)
    mockFs = vi.mocked(fs)
    mockGetClaudeEventsFilePath = vi.mocked(claudeCodeEvents.getClaudeEventsFilePath)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('createMessageService', () => {
    it('should create a MessageService instance', () => {
      const instance = createMessageService(mockClaudeCodeSettings)
      expect(instance).toBeInstanceOf(MessageService)
    })
  })

  describe('readMessagesFromClaudeDirectory', () => {
    it('should use stored settings to read messages', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockFs.readFile.mockResolvedValue('')

      const result = await service.readMessagesFromClaudeDirectory(mockTask)

      expect(result).toEqual([])
    })
  })

  describe('readMessagesFromClaudeDirectoryWithSettings', () => {
    it('should return empty array when no file path is available', async () => {
      const taskWithoutWorkspace = { ...mockTask, workspacePath: undefined }

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        taskWithoutWorkspace
      )

      expect(result).toEqual([])
    })

    it('should return empty array when file does not exist', async () => {
      const enoentError = new Error('File not found')
      ;(enoentError as any).code = 'ENOENT'
      mockFs.readFile.mockRejectedValue(enoentError)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toEqual([])
    })

    it('should throw error for other file system errors', async () => {
      const error = new Error('Permission denied')
      mockFs.readFile.mockRejectedValue(error)

      await expect(
        service.readMessagesFromClaudeDirectoryWithSettings(mockClaudeCodeSettings, mockTask)
      ).rejects.toThrow('Permission denied')
    })

    it('should parse and transform valid SDK messages', async () => {
      const sdkUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Hello, how are you?',
        },
        timestamp: 1234567890000,
      }

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
      }

      const fileContent = [
        JSON.stringify(sdkUserMessage),
        JSON.stringify(sdkAssistantMessage),
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(2)
      expect(result[0]).toEqual({
        id: expect.stringContaining('1234567890000'),
        role: 'user',
        content: 'Hello, how are you?',
        timestamp: new Date(1234567890000),
        eventId: expect.stringContaining('1234567890000'),
      })
      expect(result[1]).toEqual({
        id: expect.stringContaining('1234567890001'),
        role: 'assistant',
        content: 'I am doing well, thank you!',
        timestamp: new Date(1234567890001),
        eventId: expect.stringContaining('1234567890001'),
      })
    })

    it('should handle tool use messages with results', async () => {

      const toolUseMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'Let me check the files.',
            },
            {
              type: 'tool_use',
              id: 'tool-123',
              name: 'read_file',
              input: { file_path: 'test.txt' },
            },
          ],
        },
        timestamp: 1234567890000,
      }

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
      }

      const fileContent = [
        JSON.stringify(toolUseMessage),
        JSON.stringify(toolResultMessage),
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      expect(Array.isArray(result[0].content)).toBe(true)
      const content = result[0].content as any[]
      expect(content).toHaveLength(2)
      expect(content[0]).toEqual({
        type: 'text',
        text: 'Let me check the files.',
      })
      expect(content[1]).toEqual({
        type: 'tool_use',
        tool_name: 'read_file',
        tool_input: { file_path: 'test.txt' },
        tool_use_id: 'tool-123',
        tool_result: {
          content: 'File content here',
          is_error: false,
          eventId: expect.stringContaining('1234567890001'),
        },
      })
    })

    it('should handle tool use messages without results', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const toolUseMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              id: 'tool-456',
              name: 'read_file',
              input: { file_path: 'missing.txt' },
            },
          ],
        },
        timestamp: 1234567890000,
      }

      const fileContent = JSON.stringify(toolUseMessage)
      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].role).toBe('assistant')
      const content = result[0].content as any[]
      expect(content).toHaveLength(1)
      expect(content[0]).toEqual({
        type: 'tool_use',
        tool_name: 'read_file',
        tool_input: { file_path: 'missing.txt' },
        tool_use_id: 'tool-456',
        tool_result: undefined,
      })
    })

    it('should handle complex tool result content', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const toolUseMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              id: 'tool-789',
              name: 'bash',
              input: { command: 'ls -la' },
            },
          ],
        },
        timestamp: 1234567890000,
      }

      const toolResultMessage: SDKMessage = {
        message: {
          role: 'user',
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-789',
              content: [
                { type: 'text', text: 'total 8' },
                { type: 'text', text: 'drwxr-xr-x 2 user user 4096 Jan 1 12:00 .' },
              ],
              is_error: false,
            },
          ],
        },
        timestamp: 1234567890001,
      }

      const fileContent = [
        JSON.stringify(toolUseMessage),
        JSON.stringify(toolResultMessage),
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      const content = result[0].content as any[]
      expect(content[0].tool_result?.content).toBe('total 8\\ndrwxr-xr-x 2 user user 4096 Jan 1 12:00 .')
    })

    it('should handle error tool results', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const toolUseMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              id: 'tool-error',
              name: 'bash',
              input: { command: 'invalid-command' },
            },
          ],
        },
        timestamp: 1234567890000,
      }

      const toolResultMessage: SDKMessage = {
        message: {
          role: 'user',
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'tool-error',
              content: 'Command not found',
              is_error: true,
            },
          ],
        },
        timestamp: 1234567890001,
      }

      const fileContent = [
        JSON.stringify(toolUseMessage),
        JSON.stringify(toolResultMessage),
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      const content = result[0].content as any[]
      expect(content[0].tool_result?.is_error).toBe(true)
      expect(content[0].tool_result?.content).toBe('Command not found')
    })

    it('should skip empty user messages', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const emptyUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: '',
        },
        timestamp: 1234567890000,
      }

      const validUserMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Valid message',
        },
        timestamp: 1234567890001,
      }

      const fileContent = [
        JSON.stringify(emptyUserMessage),
        JSON.stringify(validUserMessage),
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Valid message')
    })

    it('should handle malformed JSON gracefully', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const validMessage: SDKMessage = {
        message: {
          role: 'user',
          content: 'Valid message',
        },
        timestamp: 1234567890000,
      }

      const fileContent = [
        '{ invalid json',
        JSON.stringify(validMessage),
        '{ "incomplete": ',
      ].join('\\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Valid message')
    })

    it('should handle assistant messages with single text content', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const assistantMessage: SDKMessage = {
        message: {
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'Single text response',
            },
          ],
        },
        timestamp: 1234567890000,
      }

      const fileContent = JSON.stringify(assistantMessage)
      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Single text response')
    })

    it('should handle user messages with array content', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const userMessage: SDKMessage = {
        message: {
          role: 'user',
          content: [
            { type: 'text', text: 'First part' },
            { type: 'text', text: 'Second part' },
          ],
        },
        timestamp: 1234567890000,
      }

      const fileContent = JSON.stringify(userMessage)
      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('First part\\nSecond part')
    })

    it('should handle messages without timestamps', async () => {
      const mockFilePath = '/test/claude-events.jsonl'
      mockGetClaudeEventsFilePath.mockReturnValue(mockFilePath)

      const messageWithoutTimestamp: SDKMessage = {
        message: {
          role: 'user',
          content: 'Message without timestamp',
        },
      }

      const fileContent = JSON.stringify(messageWithoutTimestamp)
      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].timestamp).toBeInstanceOf(Date)
      expect(result[0].content).toBe('Message without timestamp')
    })
  })
})
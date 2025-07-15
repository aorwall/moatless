import { promises as fs } from 'node:fs'
import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { ClaudeCodeSettings, Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageService } from '../message.service.js'

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

describe('MessageService Unit Tests', () => {
  let service: MessageService
  let mockClaudeCodeSettings: ClaudeCodeSettings
  let mockTask: Task
  let mockFs: any

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
      ].join('\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

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
      ].join('\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

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
      }

      const fileContent = [
        '{ invalid json',
        JSON.stringify(validMessage),
        '{ "incomplete": ',
      ].join('\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

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
      ].join('\n')

      mockFs.readFile.mockResolvedValue(fileContent)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toHaveLength(1)
      expect(result[0].content).toBe('Valid message')
    })

    it('should handle error conditions', async () => {
      const enoentError = new Error('File not found')
      ;(enoentError as any).code = 'ENOENT'
      mockFs.readFile.mockRejectedValue(enoentError)

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        mockTask
      )

      expect(result).toEqual([])
    })
  })

  describe('Edge Cases', () => {
    it('should handle task without workspace path', async () => {
      const taskWithoutWorkspace = { ...mockTask, workspacePath: undefined }

      const result = await service.readMessagesFromClaudeDirectoryWithSettings(
        mockClaudeCodeSettings,
        taskWithoutWorkspace
      )

      expect(result).toEqual([])
    })

    it('should handle messages without timestamps', async () => {
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
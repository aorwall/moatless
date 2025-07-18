import type { RepositoryConfig, Task } from '@moatless/types'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { ClaudeService } from '../../agents/claude-code/claude.service.js'
import { fileExists } from '../../agents/claude-code/claude-utils.js'
import { createMessageService, MessageService } from '../message.service.js'
import { initializeSettingsService } from '../settings.service.js'
import type { TaskService } from '../task.service.js'

describe('MessageService Integration - Real Session Data', () => {
  let claudeService: ClaudeService
  let messageService: MessageService
  let sessionExists: boolean
  let mockTaskService: TaskService

  const sessionId = '58a94d13-647a-4d38-b628-fa057f26ec91'
  const repositoryConfig: RepositoryConfig = {
    id: 'test-repo',
    name: 'Test Repository',
    path: '/Users/albert/repos/moatless/moatless-swarm-ui',
    description: 'Test repository for message service integration',
    codeExecution: {
      type: 'docker',
      image: 'ubuntu:latest',
    },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  beforeAll(async () => {
    const settingsService = await initializeSettingsService()
    claudeService = new ClaudeService(settingsService)

    // Create a mock TaskService
    mockTaskService = {
      getTask: vi.fn(),
      // Add other required methods as stubs if needed
    } as any

    messageService = createMessageService(claudeService, mockTaskService, repositoryConfig)

    // Check if the session file exists
    const homeDir = require('node:os').homedir()
    const sessionFilePath = require('node:path').join(
      homeDir,
      '.claude',
      'projects',
      '-Users-albert-repos-moatless-moatless-swarm-ui',
      `${sessionId}.jsonl`,
    )
    sessionExists = await fileExists(sessionFilePath)
  })

  describe('readMessagesFromClaudeDirectory', () => {
    it('should read and transform messages from real Claude session file', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      // Create a mock task with the real session ID
      const mockTask: Task = {
        id: 'test-task-message-service',
        taskId: 'test-task-message-service',
        title: 'Test Task for Message Service',
        description: 'Testing message service integration',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: sessionId, // This is the key - using the real session ID
            startTime: new Date('2025-07-18T07:01:47.948Z'),
            endTime: new Date('2025-07-18T07:02:07.457Z'),
            status: 'completed',
            messageCount: 10,
            costUsd: 0,
            initialMessage: 'try to commit again',
            result: 'Task completed successfully',
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      const messages = await messageService.readMessagesFromClaudeDirectory(mockTask)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBeGreaterThan(0)

      // Verify message structure
      const firstMessage = messages[0]
      expect(firstMessage).toHaveProperty('id')
      expect(firstMessage).toHaveProperty('role')
      expect(firstMessage).toHaveProperty('content')
      expect(firstMessage).toHaveProperty('timestamp')
      expect(firstMessage).toHaveProperty('eventId')

      // Check for both user and assistant messages
      const userMessages = messages.filter((msg) => msg.role === 'user')
      const assistantMessages = messages.filter((msg) => msg.role === 'assistant')

      expect(userMessages.length).toBeGreaterThan(0)
      expect(assistantMessages.length).toBeGreaterThan(0)

      console.log('Messages transformed successfully:', {
        totalMessages: messages.length,
        userMessages: userMessages.length,
        assistantMessages: assistantMessages.length,
        firstMessageRole: firstMessage.role,
        firstMessageContent:
          typeof firstMessage.content === 'string'
            ? firstMessage.content.slice(0, 100)
            : 'Complex content',
      })
    })

    it('should handle tool use messages correctly', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      const mockTask: Task = {
        id: 'test-task-tools',
        taskId: 'test-task-tools',
        title: 'Test Task for Tool Messages',
        description: 'Testing tool use message handling',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: sessionId,
            startTime: new Date(),
            status: 'completed',
            messageCount: 5,
            costUsd: 0,
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      const messages = await messageService.readMessagesFromClaudeDirectory(mockTask)

      // Look for messages with tool use
      const messagesWithTools = messages.filter((msg) => {
        if (Array.isArray(msg.content)) {
          return msg.content.some((content) => content.type === 'tool_use')
        }
        return false
      })

      if (messagesWithTools.length > 0) {
        const toolMessage = messagesWithTools[0]
        expect(toolMessage.role).toBe('assistant')
        expect(Array.isArray(toolMessage.content)).toBe(true)

        const toolUseContent = (toolMessage.content as any[]).find(
          (content) => content.type === 'tool_use',
        )
        expect(toolUseContent).toBeDefined()
        expect(toolUseContent).toHaveProperty('tool_name')
        expect(toolUseContent).toHaveProperty('tool_input')
        expect(toolUseContent).toHaveProperty('tool_use_id')

        console.log('Tool use message found:', {
          toolName: toolUseContent.tool_name,
          hasToolResult: !!toolUseContent.tool_result,
          toolInputKeys: Object.keys(toolUseContent.tool_input || {}),
        })
      } else {
        console.log('No tool use messages found in this session')
      }
    })

    it('should fall back to task iterations when Claude files are not accessible', async () => {
      // Create a task with a non-existent session ID
      const mockTask: Task = {
        id: 'test-task-fallback',
        taskId: 'test-task-fallback',
        title: 'Test Task for Fallback',
        description: 'Testing fallback to task iterations',
        status: 'completed',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'non-existent-session-id',
            startTime: new Date(),
            endTime: new Date(),
            status: 'completed',
            messageCount: 2,
            costUsd: 0.05,
            initialMessage: 'Test user message',
            result: JSON.stringify({
              content: 'Test assistant response from iteration',
              type: 'completion',
            }),
          },
          {
            iterationNumber: 2,
            agentSessionId: 'another-non-existent-session',
            startTime: new Date(),
            endTime: new Date(),
            status: 'completed',
            messageCount: 1,
            costUsd: 0.03,
            initialMessage: 'Another test message',
            result: 'Simple text result from second iteration',
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 2,
        totalCostUsd: 0.08,
      }

      const messages = await messageService.readMessagesFromClaudeDirectory(mockTask)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBeGreaterThan(0)

      // Should have messages from both iterations
      expect(messages.length).toBe(4) // 2 iterations × 2 messages each (user + assistant)

      // Verify first iteration messages
      const firstUserMessage = messages.find(
        (msg) => msg.id.includes('iteration-1-user') && msg.role === 'user',
      )
      const firstAssistantMessage = messages.find(
        (msg) => msg.id.includes('iteration-1-assistant') && msg.role === 'assistant',
      )

      expect(firstUserMessage).toBeDefined()
      expect(firstUserMessage?.content).toBe('Test user message')

      expect(firstAssistantMessage).toBeDefined()
      expect(firstAssistantMessage?.content).toBe('Test assistant response from iteration')

      // Verify second iteration messages
      const secondUserMessage = messages.find(
        (msg) => msg.id.includes('iteration-2-user') && msg.role === 'user',
      )
      const secondAssistantMessage = messages.find(
        (msg) => msg.id.includes('iteration-2-assistant') && msg.role === 'assistant',
      )

      expect(secondUserMessage).toBeDefined()
      expect(secondUserMessage?.content).toBe('Another test message')

      expect(secondAssistantMessage).toBeDefined()
      expect(secondAssistantMessage?.content).toBe('Simple text result from second iteration')

      console.log('Fallback to task iterations successful:', {
        totalMessages: messages.length,
        iterationsProcessed: 2,
        messageTypes: messages.map((msg) => msg.role),
      })
    })
  })

  describe('message transformation edge cases', () => {
    it('should handle complex message content structures', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      const mockTask: Task = {
        id: 'test-task-complex',
        taskId: 'test-task-complex',
        title: 'Test Complex Messages',
        description: 'Testing complex message structures',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: sessionId,
            startTime: new Date(),
            status: 'completed',
            messageCount: 3,
            costUsd: 0,
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      const messages = await messageService.readMessagesFromClaudeDirectory(mockTask)

      // Verify that all messages have proper timestamps
      for (const message of messages) {
        expect(message.timestamp).toBeInstanceOf(Date)
        expect(message.timestamp.getTime()).toBeGreaterThan(0)
      }

      // Verify that all messages have unique IDs
      const messageIds = messages.map((msg) => msg.id)
      const uniqueIds = new Set(messageIds)
      expect(uniqueIds.size).toBe(messageIds.length)

      // Check for consistent eventId format
      for (const message of messages) {
        expect(typeof message.eventId).toBe('string')
        expect(message.eventId.length).toBeGreaterThan(0)
      }

      console.log('Message structure validation passed:', {
        allHaveTimestamps: messages.every((msg) => msg.timestamp instanceof Date),
        allHaveUniqueIds: uniqueIds.size === messageIds.length,
        allHaveEventIds: messages.every((msg) => typeof msg.eventId === 'string'),
      })
    })

    it('should handle empty or malformed iterations gracefully', async () => {
      const mockTaskWithEmptyIterations: Task = {
        id: 'test-task-empty',
        taskId: 'test-task-empty',
        title: 'Test Empty Iterations',
        description: 'Testing empty iteration handling',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'non-existent',
            startTime: new Date(),
            status: 'created',
            messageCount: 0,
            costUsd: 0,
            // No initialMessage or result
          },
          {
            iterationNumber: 2,
            agentSessionId: 'another-non-existent',
            startTime: new Date(),
            status: 'created',
            messageCount: 0,
            costUsd: 0,
            initialMessage: '', // Empty message
            result: '', // Empty result
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 0,
        totalCostUsd: 0,
      }

      const messages = await messageService.readMessagesFromClaudeDirectory(
        mockTaskWithEmptyIterations,
      )

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      // Should return empty array or minimal messages since iterations are empty
      expect(messages.length).toBe(0)

      console.log('Empty iterations handled gracefully - no messages created')
    })
  })

  describe('getTaskMessages', () => {
    it('should get messages using task ID', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      const taskId = 'test-task-get-messages'
      const mockTask: Task = {
        id: taskId,
        taskId: taskId,
        title: 'Test Task for Get Messages',
        description: 'Testing getTaskMessages method',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: sessionId,
            startTime: new Date(),
            status: 'completed',
            messageCount: 5,
            costUsd: 0,
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Mock the TaskService to return our task
      vi.mocked(mockTaskService.getTask).mockResolvedValue(mockTask)

      // Test the new method
      const messages = await messageService.getTaskMessages(taskId)

      expect(mockTaskService.getTask).toHaveBeenCalledWith(taskId)
      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBeGreaterThan(0)

      // Verify message structure
      const firstMessage = messages[0]
      expect(firstMessage).toHaveProperty('id')
      expect(firstMessage).toHaveProperty('role')
      expect(firstMessage).toHaveProperty('content')

      console.log('getTaskMessages successful:', {
        taskId,
        messageCount: messages.length,
        firstMessageRole: firstMessage.role,
      })
    })

    it('should throw error for non-existent task ID', async () => {
      const nonExistentTaskId = 'non-existent-task-id-12345'

      // Mock the TaskService to return null
      vi.mocked(mockTaskService.getTask).mockResolvedValue(null)

      await expect(messageService.getTaskMessages(nonExistentTaskId)).rejects.toThrow(
        `Task not found: ${nonExistentTaskId}`,
      )

      expect(mockTaskService.getTask).toHaveBeenCalledWith(nonExistentTaskId)
    })

    it('should throw error for non-Claude Code tasks', async () => {
      const taskId = 'non-claude-task'
      const mockTask: Task = {
        id: taskId,
        taskId: taskId,
        title: 'Non-Claude Task',
        description: 'Testing with non-Claude agent type',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'some-other-agent', // Not claude-code
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Mock the TaskService to return our task
      vi.mocked(mockTaskService.getTask).mockResolvedValue(mockTask)

      await expect(messageService.getTaskMessages(taskId)).rejects.toThrow(
        'Only Claude Code tasks are supported for message reading',
      )
    })

    it('should handle task without Claude session', async () => {
      const taskId = 'test-task-no-session'
      const mockTask: Task = {
        id: taskId,
        taskId: taskId,
        title: 'Test Task Without Session',
        description: 'Testing task without Claude session',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'non-existent-session',
            startTime: new Date(),
            status: 'completed',
            messageCount: 2,
            costUsd: 0,
            initialMessage: 'Test message without session',
            result: 'Fallback to iteration data',
          },
        ],
        createdAt: new Date(),
        updatedAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Mock the TaskService to return our task
      vi.mocked(mockTaskService.getTask).mockResolvedValue(mockTask)

      // Test the method with a task that has no Claude session
      const messages = await messageService.getTaskMessages(taskId)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(2) // Should have user and assistant messages from iteration

      const userMessage = messages.find((msg) => msg.role === 'user')
      const assistantMessage = messages.find((msg) => msg.role === 'assistant')

      expect(userMessage).toBeDefined()
      expect(userMessage?.content).toBe('Test message without session')

      expect(assistantMessage).toBeDefined()
      expect(assistantMessage?.content).toBe('Fallback to iteration data')

      console.log('getTaskMessages with fallback successful:', {
        taskId,
        messageCount: messages.length,
        fallbackUsed: true,
      })
    })
  })

  describe('service factory function', () => {
    it('should create MessageService instance with factory function', () => {
      const service = createMessageService(claudeService, mockTaskService, repositoryConfig)

      expect(service).toBeInstanceOf(MessageService)
      expect(service).toBeDefined()
    })

    it('should create MessageService instance without repository config', () => {
      const service = createMessageService(claudeService, mockTaskService)

      expect(service).toBeInstanceOf(MessageService)
      expect(service).toBeDefined()
    })
  })
})

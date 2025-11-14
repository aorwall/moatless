import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { AgentMessage, Task as TaskType } from '@moatless/types'
import { Container } from 'inversify'

// Alias for compatibility with existing tests
type TaskResponse = TaskType

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../../infrastructure/database/database'
import { createDrizzleDatabase } from '../../../../infrastructure/database/drizzle'
import { FileStorage } from '../../../../infrastructure/storage/file-storage'
import { InfrastructureStorageSymbol } from '../../../../infrastructure/storage/storage'
import { fileExists } from '../../../claude/agent/claude-utils.ts'
import { ClaudeService } from '../../../claude/index'
import { FileStorageService } from '../../../file-storage'
import { TaskRepository } from '../../../tasks/db/task.repository'
import { TaskMessageRepository } from '../../../tasks/db/task-message.repository'
import { MessageService } from '../message.service'

describe('MessageService Integration - Real File Storage', () => {
  let container: Container
  let claudeService: ClaudeService
  let messageService: MessageService
  let fileStorage: FileStorage
  let sessionExists: boolean
  let tempTestDir: string
  let database: Database
  let taskRepository: TaskRepository
  let taskMessageRepository: TaskMessageRepository

  const sessionId = '58a94d13-647a-4d38-b628-fa057f26ec91'

  beforeAll(async () => {
    // Create a temporary directory for test storage
    tempTestDir = path.join(os.tmpdir(), 'message-service-integration-test', randomUUID())
    await fs.mkdir(tempTestDir, { recursive: true })

    // Create DI container
    container = new Container()

    // Create real file storage
    fileStorage = new FileStorage(tempTestDir)

    // Bind storage to container
    container.bind(InfrastructureStorageSymbol).toConstantValue(fileStorage)

    // Create relational database for message repository
    const dbPath = path.join(tempTestDir, 'messages.db')
    database = createDrizzleDatabase(dbPath)
    await database.migrate()
    container.bind(DatabaseSymbol).toConstantValue(database)

    // Bind storage adapters
    container.bind(TaskRepository).toSelf()
    container.bind(TaskMessageRepository).toSelf()

    // Mock ClaudeService
    claudeService = {
      readClaudeMessages: vi.fn().mockResolvedValue([]),
    } as any
    container.bind(ClaudeService).toConstantValue(claudeService)

    // Mock file storage service
    const mockUploadService = {
      uploadFile: vi.fn(),
      getFile: vi.fn(),
      getFileMetadata: vi.fn(),
      deleteFile: vi.fn(),
    } as any
    container.bind(FileStorageService).toConstantValue(mockUploadService)

    // Bind and resolve MessageService
    container.bind(MessageService).toSelf()
    taskRepository = container.get(TaskRepository)
    taskMessageRepository = container.get(TaskMessageRepository)
    messageService = container.get(MessageService)

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

  afterAll(async () => {
    if (database) {
      await database.close()
    }

    // Clean up temporary test directory
    try {
      await fs.rm(tempTestDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Failed to clean up temp test directory:', error)
    }
  })

  describe('readMessagesFromClaudeDirectory', () => {
    it('should read and transform messages from real Claude session file', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      // Create a mock task with the real session ID
      const mockTask: TaskResponse = {
        taskId: 'test-task-message-service',
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
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
      if (!firstMessage) throw new Error('Expected first message')
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

      const mockTask: TaskResponse = {
        taskId: 'test-task-tools',
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
        if (!toolMessage) throw new Error('Expected tool message')
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
  })

  describe('message transformation edge cases', () => {
    it('should handle complex message content structures', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      const mockTask: TaskResponse = {
        taskId: 'test-task-complex',
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
        if (message.eventId) {
          expect(typeof message.eventId).toBe('string')
          expect(message.eventId.length).toBeGreaterThan(0)
        }
      }

      console.log('Message structure validation passed:', {
        allHaveTimestamps: messages.every((msg) => msg.timestamp instanceof Date),
        allHaveUniqueIds: uniqueIds.size === messageIds.length,
        allHaveEventIds: messages.every((msg) => !msg.eventId || typeof msg.eventId === 'string'),
      })
    })

    it('should handle empty or malformed iterations gracefully', async () => {
      const mockTaskWithEmptyIterations: TaskResponse = {
        taskId: 'test-task-empty',
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
    it('should get messages using task ID (optional - requires real Claude session)', async () => {
      if (!sessionExists) {
        console.log('Skipping test - session file not found')
        return
      }

      const taskId = 'test-task-get-messages'
      const mockTask: TaskResponse = {
        taskId: taskId,
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
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Save the task to database so TaskService can find it
      await taskRepository.save(mockTask)

      // Test the new method
      const messages = await messageService.getTaskMessages(taskId)
      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBeGreaterThan(0)

      // Verify message structure
      const firstMessage = messages[0]
      if (!firstMessage) throw new Error('Expected first message')
      expect(firstMessage).toHaveProperty('id')
      expect(firstMessage).toHaveProperty('role')
      expect(firstMessage).toHaveProperty('content')
      expect(firstMessage).toHaveProperty('timestamp')

      console.log('getTaskMessages successful:', {
        taskId,
        messageCount: messages.length,
        firstMessageRole: firstMessage.role,
      })
    })

    it('should throw error for non-existent task ID', async () => {
      const nonExistentTaskId = 'non-existent-task-id-12345'

      // Don't save any task, so it won't be found
      await expect(messageService.getTaskMessages(nonExistentTaskId)).rejects.toThrow(
        `Task ${nonExistentTaskId} not found`,
      )
    })

    it('should throw error for non-Claude Code tasks', async () => {
      const taskId = 'non-claude-task'
      const mockTask: TaskResponse = {
        taskId: taskId,
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'some-other-agent', // Not claude-code
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Save the task to storage
      await taskRepository.save(mockTask)

      await expect(messageService.getTaskMessages(taskId)).rejects.toThrow(
        'Only Claude Code tasks are supported for message reading',
      )
    })

    it('should handle task without Claude session', async () => {
      const taskId = 'test-task-no-session'
      const mockTask: TaskResponse = {
        taskId: taskId,
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
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Save the task to storage
      await taskRepository.save(mockTask)

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
      if (Array.isArray(assistantMessage?.content)) {
        expect(assistantMessage?.content[0]?.text).toBe('Fallback to iteration data')
      } else {
        expect(assistantMessage?.content).toBe('Fallback to iteration data')
      }

      console.log('getTaskMessages with fallback successful:', {
        taskId,
        messageCount: messages.length,
        fallbackUsed: true,
      })
    })
  })

  describe('Message repository persistence through AgentService', () => {
    it('should save and retrieve user messages through the service workflow', async () => {
      const taskId = 'test-user-message-persistence'

      // Create a task with iteration data
      const mockTask: TaskResponse = {
        taskId,
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'test-session-123',
            startTime: new Date(),
            status: 'active',
            messageCount: 1,
            costUsd: 0,
            initialMessage: 'Test user message for persistence',
          },
        ],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      // Save the task
      await taskRepository.save(mockTask)

      // Simulate AgentService saving a user message (this is what happens in executeAgent)
      const userMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: 'Test user message for persistence',
        },
        sessionId: 'test-session-123',
      } satisfies AgentMessage

      // Save the user message via repository to simulate persistence layer
      await taskMessageRepository.append(taskId, userMessage)

      // Now test that MessageService can retrieve the messages properly
      const messages = await messageService.getTaskMessages(taskId)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(1)

      // Verify the user message was properly transformed
      const retrievedMessage = messages[0]
      if (!retrievedMessage) throw new Error('Expected retrieved message')
      expect(retrievedMessage.role).toBe('user')
      expect(retrievedMessage.content).toBe('Test user message for persistence')
      expect(retrievedMessage.id).toBeDefined()
      expect(retrievedMessage.timestamp).toBeDefined()

      // Verify it came from the message repository (priority test)
      const messagesFromRepository = await messageService.getMessagesFromRepository(taskId)
      expect(messagesFromRepository.length).toBe(1)
      expect(messagesFromRepository[0]?.type).toBe('user')

      // Verify the raw persisted payload matches the AgentMessage structure
      const rawMessages = await taskMessageRepository.getMessages(taskId)
      expect(rawMessages.length).toBe(1)
      expect(rawMessages[0]?.uuid).toBe(userMessage.uuid)
      expect(rawMessages[0]?.message).toEqual(userMessage.message)

      console.log('User message persistence test successful:', {
        taskId,
        messagesSaved: 1,
        messagesRetrieved: messages.length,
        messageContent: `${(
          typeof retrievedMessage.content === 'string'
            ? retrievedMessage.content
            : 'Complex content'
        ).substring(0, 30)}...`,
      })
    })

    it('should save and retrieve assistant messages with correct format', async () => {
      const taskId = 'test-assistant-message-persistence'

      // Create a task
      const mockTask: TaskResponse = {
        taskId,
        status: 'running',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'test-session-456',
            startTime: new Date(),
            status: 'active',
            messageCount: 2,
            costUsd: 0,
          },
        ],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      await taskRepository.save(mockTask)

      // Simulate AgentService saving messages as they would be saved during execution
      const userMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: 'User question about code',
        },
        sessionId: 'test-session-456',
      } satisfies AgentMessage

      const assistantMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'assistant',
        type: 'assistant',
        message: {
          type: 'message',
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'I can help you with that code. Let me take a look.',
            },
          ],
        },
        sessionId: 'test-session-456',
      } satisfies AgentMessage

      // Save messages in the order they would be created
      await taskMessageRepository.append(taskId, userMessage)
      await taskMessageRepository.append(taskId, assistantMessage)

      // Test MessageService retrieval and transformation
      const messages = await messageService.getTaskMessages(taskId)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(2)

      // Verify user message
      const retrievedUserMessage = messages.find((msg) => msg.role === 'user')
      expect(retrievedUserMessage).toBeDefined()
      expect(retrievedUserMessage?.content).toBe('User question about code')

      // Verify assistant message
      const retrievedAssistantMessage = messages.find((msg) => msg.role === 'assistant')
      expect(retrievedAssistantMessage).toBeDefined()
      // Assistant message content is an array, need to check the text content
      expect(Array.isArray(retrievedAssistantMessage?.content)).toBe(true)
      if (Array.isArray(retrievedAssistantMessage?.content)) {
        const textContent = retrievedAssistantMessage.content.find((c) => c.type === 'text')
        expect(textContent?.text).toBe('I can help you with that code. Let me take a look.')
      }

      // Verify the raw JSONL data is properly formatted
      const rawMessages = await taskMessageRepository.getMessages(taskId)
      expect(rawMessages.length).toBe(2)
      expect(rawMessages[0]?.uuid).toBe(userMessage.uuid)
      expect(rawMessages[1]?.type).toBe('assistant')
      expect(Array.isArray((rawMessages[1]?.message as any).content)).toBe(true)

      console.log('Assistant message persistence test successful:', {
        taskId,
        messagesSaved: 2,
        messagesRetrieved: messages.length,
        userMessageExists: !!retrievedUserMessage,
        assistantMessageExists: !!retrievedAssistantMessage,
      })
    })

    it('should prioritize local JSONL storage over Claude directory fallback', async () => {
      const taskId = 'test-storage-priority'

      // Create a task that would have Claude directory data
      const mockTask: TaskResponse = {
        taskId,
        status: 'closed',
        workspacePath: '/Users/albert/repos/moatless/moatless-swarm-ui',
        iterations: [
          {
            iterationNumber: 1,
            agentSessionId: 'fallback-session',
            startTime: new Date(),
            endTime: new Date(),
            status: 'completed',
            messageCount: 1,
            costUsd: 0,
            initialMessage: 'Message that would come from Claude directory',
            result: 'Result from Claude directory fallback',
          },
        ],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
      }

      await taskRepository.save(mockTask)

      // Simulate that we have messages saved through AgentService
      const localMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: 'Message saved through AgentService to local storage',
        },
        sessionId: 'fallback-session',
      } satisfies AgentMessage

      await taskMessageRepository.append(taskId, localMessage)

      // Test that MessageService prioritizes the message repository
      const messages = await messageService.getTaskMessages(taskId)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(1)

      // Should get the message from the repository, not the Claude directory fallback
      expect(messages[0]?.content).toBe('Message saved through AgentService to local storage')
      expect(messages[0]?.content).not.toBe('Message that would come from Claude directory')

      // Verify that getMessagesFromStorage actually found the local messages
      const localMessages = await messageService.getMessagesFromRepository(taskId)
      expect(localMessages.length).toBe(1)

      console.log(
        'Storage priority test successful - repository took precedence over Claude directory',
      )
    })
  })
})

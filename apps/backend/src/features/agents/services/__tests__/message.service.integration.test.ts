import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { AgentMessage, Task as TaskType } from '@moatless/types'
import { Container } from 'inversify'

// Alias for compatibility with existing tests
type TaskResponse = TaskType

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../../infrastructure/database/database'
import { createDrizzleDatabase } from '../../../../infrastructure/database/drizzle'
import { FileStorage } from '../../../../infrastructure/storage/file-storage'
import { InfrastructureStorageSymbol } from '../../../../infrastructure/storage/storage'
import { EventService } from '../../../events/services/event.service'
import { FileStorageService } from '../../../file-storage'
import { TaskRepository } from '../../../tasks/db/task.repository'
import { TaskMessageRepository } from '../../../tasks/db/task-message.repository'
import { MessageService } from '../message.service'

describe('MessageService Integration - Real File Storage', () => {
  let container: Container
  let messageService: MessageService
  let fileStorage: FileStorage
  let tempTestDir: string
  let database: Database
  let taskRepository: TaskRepository
  let taskMessageRepository: TaskMessageRepository

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

    // Mock file storage service
    const mockUploadService = {
      uploadFile: () => Promise.resolve(),
      getFile: () => Promise.resolve(null),
      getFileMetadata: () => Promise.resolve(null),
      deleteFile: () => Promise.resolve(),
    } as unknown as FileStorageService
    container.bind(FileStorageService).toConstantValue(mockUploadService)

    // Mock EventService
    const mockEventService = {
      subscribeAll: () => () => {
        /* unsubscribe noop */
      },
      emitEvent: () => {
        /* emit noop */
      },
    } as unknown as EventService
    container.bind(EventService).toConstantValue(mockEventService)

    // Bind and resolve MessageService
    container.bind(MessageService).toSelf()
    taskRepository = container.get(TaskRepository)
    taskMessageRepository = container.get(TaskMessageRepository)
    messageService = container.get(MessageService)
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

  describe('getTaskMessages', () => {
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
        visibility: 'private' as const,
      }

      // Save the task to storage
      await taskRepository.save(mockTask)

      await expect(messageService.getTaskMessages(taskId)).rejects.toThrow(
        'Only Claude Code tasks are supported for message reading',
      )
    })

    it('should fall back to task iterations when repository is empty', async () => {
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
        visibility: 'private' as const,
      }

      // Save the task to storage
      await taskRepository.save(mockTask)

      // Test the method with a task that has no messages in repository
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
        visibility: 'private' as const,
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
          content: [{ type: 'text', text: 'Test user message for persistence' }],
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
        visibility: 'private' as const,
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
          content: [{ type: 'text', text: 'User question about code' }],
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
      expect(Array.isArray((rawMessages[1]?.message as { content: unknown }).content)).toBe(true)

      console.log('Assistant message persistence test successful:', {
        taskId,
        messagesSaved: 2,
        messagesRetrieved: messages.length,
        userMessageExists: !!retrievedUserMessage,
        assistantMessageExists: !!retrievedAssistantMessage,
      })
    })

    it('should prioritize repository over task iterations fallback', async () => {
      const taskId = 'test-storage-priority'

      // Create a task that would have iteration fallback data
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
            initialMessage: 'Message that would come from iteration fallback',
            result: 'Result from iteration fallback',
          },
        ],
        createdAt: new Date(),
        lastMessageAt: new Date(),
        repositoryId: 'test-repo',
        agentType: 'claude-code',
        numTurns: 1,
        totalCostUsd: 0,
        visibility: 'private' as const,
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
          content: [{ type: 'text', text: 'Message saved through AgentService to repository' }],
        },
        sessionId: 'fallback-session',
      } satisfies AgentMessage

      await taskMessageRepository.append(taskId, localMessage)

      // Test that MessageService prioritizes the message repository
      const messages = await messageService.getTaskMessages(taskId)

      expect(messages).toBeDefined()
      expect(Array.isArray(messages)).toBe(true)
      expect(messages.length).toBe(1)

      // Should get the message from the repository, not the iteration fallback
      expect(messages[0]?.content).toBe('Message saved through AgentService to repository')
      expect(messages[0]?.content).not.toBe('Message that would come from iteration fallback')

      // Verify that getMessagesFromRepository actually found the messages
      const localMessages = await messageService.getMessagesFromRepository(taskId)
      expect(localMessages.length).toBe(1)

      console.log(
        'Storage priority test successful - repository took precedence over iteration fallback',
      )
    })
  })
})

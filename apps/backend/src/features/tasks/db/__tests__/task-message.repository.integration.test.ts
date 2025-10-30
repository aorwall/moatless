import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { AgentMessage } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { createDrizzleDatabase } from '../../../../infrastructure/database/drizzle'
import { logger } from '../../../../utils/logger'
import { TaskMessageRepository } from '../task-message.repository'

describe('TaskMessageRepository', () => {
  let repository: TaskMessageRepository
  let database: Database
  let tempDir: string
  let dbPath: string

  beforeEach(async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'task-message-repo-'))
    dbPath = path.join(tempDir, 'messages.db')
    database = createDrizzleDatabase(dbPath, logger)
    await database.migrate()
    repository = new TaskMessageRepository(database)
  })

  afterEach(async () => {
    await database.close()
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  function createUserMessage(overrides: Partial<AgentMessage> = {}): AgentMessage {
    return {
      uuid: overrides.uuid ?? `user-${randomUUID()}`,
      userType: overrides.userType ?? 'user',
      type: 'user',
      timestamp: overrides.timestamp ?? new Date().toISOString(),
      message: {
        role: 'user',
        content: 'Test user message',
      },
      sessionId: 'session-1',
      ...overrides,
    } as AgentMessage
  }

  function createAssistantMessage(overrides: Partial<AgentMessage> = {}): AgentMessage {
    return {
      uuid: overrides.uuid ?? `assistant-${randomUUID()}`,
      userType: overrides.userType ?? 'assistant',
      type: 'assistant',
      timestamp: overrides.timestamp ?? new Date().toISOString(),
      message: {
        type: 'message',
        role: 'assistant',
        content: [
          {
            type: 'text',
            text: 'Assistant response',
          },
        ],
      },
      sessionId: 'session-1',
      ...overrides,
    } as AgentMessage
  }

  it('should append and retrieve messages ordered by timestamp', async () => {
    const taskId = 'task-123'
    const earlier = createUserMessage({ timestamp: '2024-01-01T00:00:00.000Z' })
    const later = createAssistantMessage({ timestamp: '2024-01-01T00:00:01.000Z' })

    await repository.append(taskId, later)
    await repository.append(taskId, earlier)

    const messages = await repository.getMessages(taskId)

    expect(messages).toHaveLength(2)
    expect(messages[0]?.uuid).toBe(earlier.uuid)
    expect(messages[1]?.uuid).toBe(later.uuid)
  })

  it('should ignore duplicate messages based on uuid', async () => {
    const taskId = 'task-duplicate'
    const message = createUserMessage({ uuid: 'duplicate-uuid' })

    await repository.append(taskId, message)
    await repository.append(taskId, message)

    const messages = await repository.getMessages(taskId)
    expect(messages).toHaveLength(1)
  })

  it('should append many messages in one call', async () => {
    const taskId = 'task-batch'
    const user = createUserMessage({ uuid: 'batch-user' })
    const assistant = createAssistantMessage({ uuid: 'batch-assistant' })

    await repository.appendMany(taskId, [user, assistant])

    const messages = await repository.getMessages(taskId)
    expect(messages.map((m) => m?.uuid)).toEqual(['batch-user', 'batch-assistant'])
  })

  it('should persist and retrieve message contexts', async () => {
    const taskId = 'task-contexts'
    const contexts = [
      {
        type: 'element' as const,
        referenceId: 'component-1',
        data: {
          componentId: 'component-1',
          path: '/app/components/Button.tsx',
          line: 12,
          column: 5,
          componentName: 'Button',
        },
      },
      {
        type: 'figma' as const,
        referenceId: 'import-1',
        data: {
          importId: 'import-1',
          fileName: 'Landing Page.fig',
          nodeName: 'Hero Section',
          hasImage: true,
        },
      },
    ]

    const message = createUserMessage({ uuid: 'context-message', contexts })

    await repository.append(taskId, message)

    const [stored] = await repository.getMessages(taskId)

    expect(stored?.type).toBe('user')
    if (!stored || stored.type !== 'user') {
      throw new Error('Expected stored message to be a user message')
    }

    expect(stored.contexts).toEqual(contexts)
  })

  it('should delete all messages for a task', async () => {
    const taskId = 'task-delete'
    await repository.append(taskId, createUserMessage())

    expect(await repository.hasMessages(taskId)).toBe(true)

    await repository.deleteByTask(taskId)

    const messages = await repository.getMessages(taskId)
    expect(messages).toHaveLength(0)
    expect(await repository.hasMessages(taskId)).toBe(false)
  })

  it('should persist assistant metadata fields outside of content', async () => {
    const taskId = 'task-assistant-metadata'
    const assistant = createAssistantMessage({
      message: {
        id: 'msg-123',
        type: 'message',
        role: 'assistant',
        model: 'claude-3-5',
        stop_reason: null,
        stop_sequence: 'end',
        usage: {
          input_tokens: 42,
          output_tokens: 128,
          cache_creation_input_tokens: 10,
          cache_read_input_tokens: 4,
        },
        content: [
          {
            type: 'text',
            text: 'Assistant with metadata',
          },
        ],
      },
      requestId: 'req-42',
    })

    await repository.append(taskId, assistant)

    const [stored] = await repository.getMessages(taskId)

    expect(stored).toEqual(assistant)
  })
})
